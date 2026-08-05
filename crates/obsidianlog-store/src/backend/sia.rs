//! Sia-backed [`StorageBackend`] via the `sia_storage` SDK (the indexd app API).
//!
//! Compiled only under the `sia` feature; the pre-1.0 SDK never enters a default
//! build (see ADR-0006). The SDK brings its own `reqwest` configured with
//! `rustls-tls` (no native-tls / OpenSSL).
//!
//! ## Mapping our paths onto a content-addressed store
//!
//! `sia_storage` is **content-addressed**: uploads return an [`Object`] keyed by
//! a derived hash, and there is no get-by-path. Each object, however, carries an
//! arbitrary `metadata: Vec<u8>` blob. We store a small JSON envelope there — the
//! object's bucket-relative path (`<bucket>/chunks/<service>/<window>-<sequence>.bin`, etc.)
//! plus, for archive objects, the window's index — and resolve reads by scanning
//! the indexer's object list (`object_events`) for a matching path. A chunk and
//! its index are therefore **one object** (ADR-0008): the ciphertext is the body
//! and the index rides in metadata, so `list_chunks`/`get_index` read the index
//! without downloading any bodies. This keeps `LocalBackend` and `SiaBackend`
//! interchangeable behind [`StorageBackend`].
//!
//! Lookups still scan `object_events` (`O(objects)`); recording each chunk's
//! object id in the manifest for direct `Sdk::object` fetch is a future
//! optimization. Objects are also encrypted by the SDK, on top of our own
//! AES-256-GCM (defense in depth).

use std::io::Cursor;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sia_storage::{
    AppKey, AppMetadata, Builder, DownloadOptions, Object, Sdk, UploadOptions, app_id,
};
use tokio::io::AsyncReadExt;

use super::{decode_chunk, encode_chunk, overlaps};
use obsidianlog_core::backend::{StorageBackend, TimeRange};
use obsidianlog_core::chunk::{Chunk, ChunkRef};
use obsidianlog_core::error::{Error, Result};
use obsidianlog_core::index::ServiceWindowIndex;
use obsidianlog_core::manifest::Manifest;

/// Application metadata registered with the indexer.
///
/// `id` is ObsidianLog's stable App ID (generated once with `openssl rand -hex
/// 32`). It is **public, not a secret** — it is an input to the per-user app-key
/// derivation, so the onboarding tool and this backend must use the *same* value
/// (see the `onboard` example). `name`/`description`/etc. are display-only in the
/// approval UI.
pub const APP_META: AppMetadata = AppMetadata {
    id: app_id!("599c3cd5a89ba1a569c2f3771a1d1b066b139dfde888c74e60d7885a54d49ae6"),
    name: "ObsidianLog",
    description: "Long-term, tamper-evident operational log archival on Sia.",
    service_url: "https://github.com/emmaglorypraise/ObsidianLog",
    logo_url: None,
    callback_url: None,
};

/// Connection settings for a Sia indexer (indexd) deployment.
#[derive(Clone)]
pub struct SiaConfig {
    /// Base URL of the indexer.
    pub indexer_url: String,
    /// Bucket / namespace prefix for object paths.
    pub bucket: String,
    /// The 32-byte `AppKey` from a prior approved connection (see `AppKey::export`).
    pub app_key: [u8; 32],
}

/// Sia-backed [`StorageBackend`]. See the module docs for the path mapping.
pub struct SiaBackend {
    sdk: Sdk,
    bucket: String,
}

/// The JSON envelope stored in each object's `metadata` blob: the object's
/// logical path (Sia is content-addressed, so we match on this) and, for archive
/// objects, the window's index — letting listing and prefilter read the index
/// without downloading the body. If an index ever exceeds Sia's metadata limit,
/// fall back to a separate index object (ADR-0008).
#[derive(Serialize, Deserialize)]
struct ObjectMeta {
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    index: Option<ServiceWindowIndex>,
}

impl SiaBackend {
    /// Connect to the indexer using a previously approved [`AppKey`].
    ///
    /// Returns an error if the key has not been approved by the indexer (the
    /// interactive first-time approval flow is out of scope for the backend).
    pub async fn connect(config: SiaConfig) -> Result<Self> {
        let builder = Builder::new(config.indexer_url.as_str(), APP_META)
            .map_err(|e| Error::Backend(format!("sia builder: {e}")))?;
        let app_key = AppKey::import(config.app_key);
        let sdk = builder
            .connected(&app_key)
            .await
            .map_err(|e| Error::Backend(format!("sia connect: {e}")))?
            .ok_or_else(|| {
                Error::Backend("sia app key is not approved by the indexer".to_string())
            })?;
        Ok(Self {
            sdk,
            bucket: config.bucket,
        })
    }

    fn object_path(&self, kind: &str, service: &str, window: &str, ext: &str) -> String {
        format!("{}/{kind}/{service}/{window}.{ext}", self.bucket)
    }

    /// `(service, window)` is not a unique object path — a window can hold
    /// many chunks over its lifetime, one per batch that touches it — so the
    /// path is keyed on `sequence` too (unique and monotonic per service).
    fn chunk_path(&self, service: &str, window: &str, sequence: u64) -> String {
        self.object_path("chunks", service, &format!("{window}-{sequence}"), "bin")
    }

    fn manifest_path(&self) -> String {
        format!("{}/manifest.json", self.bucket)
    }

    /// Upload `bytes` as an object carrying `meta` in its metadata blob, then pin.
    async fn put_object(&self, meta: &ObjectMeta, bytes: Vec<u8>) -> Result<()> {
        let mut object = Object::default();
        object.metadata = serde_json::to_vec(meta)?;
        let object = self
            .sdk
            .upload(object, Cursor::new(bytes), UploadOptions::default())
            .await
            .map_err(|e| Error::Backend(format!("sia upload: {e}")))?;
        self.sdk
            .pin_object(&object)
            .await
            .map_err(|e| Error::Backend(format!("sia pin: {e}")))?;
        Ok(())
    }

    /// Parse an object's [`ObjectMeta`] from its metadata blob, if present.
    fn object_meta(object: &Object) -> Option<ObjectMeta> {
        serde_json::from_slice(&object.metadata).ok()
    }

    /// Find the newest non-deleted object whose metadata path equals `path`.
    async fn find_object(&self, path: &str) -> Result<Option<Object>> {
        let events = self
            .sdk
            .object_events(None, None)
            .await
            .map_err(|e| Error::Backend(format!("sia list: {e}")))?;

        let mut best: Option<Object> = None;
        for event in events {
            if event.deleted {
                continue;
            }
            let Some(object) = event.object else { continue };
            if Self::object_meta(&object).is_none_or(|m| m.path != path) {
                continue;
            }
            let newer = best
                .as_ref()
                .is_none_or(|b| object.updated_at() >= b.updated_at());
            if newer {
                best = Some(object);
            }
        }
        Ok(best)
    }

    /// Download an object's full contents.
    async fn read_object(&self, object: &Object) -> Result<Vec<u8>> {
        let mut download = self
            .sdk
            .download(object, DownloadOptions::default())
            .map_err(|e| Error::Backend(format!("sia download: {e}")))?;
        let mut buf = Vec::new();
        download
            .read_to_end(&mut buf)
            .await
            .map_err(|e| Error::Backend(format!("sia read: {e}")))?;
        Ok(buf)
    }

    /// Fetch an object's bytes by path, mapping absence to [`Error::NotFound`].
    async fn get_object(&self, path: &str, describe: impl FnOnce() -> String) -> Result<Vec<u8>> {
        match self.find_object(path).await? {
            Some(object) => self.read_object(&object).await,
            None => Err(Error::NotFound(describe())),
        }
    }
}

#[async_trait]
impl StorageBackend for SiaBackend {
    async fn put_archive(&self, chunk: &Chunk, index: &ServiceWindowIndex) -> Result<()> {
        // One object per (service, window, sequence): the ciphertext frame is
        // the body; the index rides in the object metadata so reads/lists
        // never download the body.
        let meta = ObjectMeta {
            path: self.chunk_path(
                &chunk.header.service,
                &chunk.header.time_window,
                chunk.header.sequence,
            ),
            index: Some(index.clone()),
        };
        self.put_object(&meta, encode_chunk(chunk)?).await
    }

    async fn get_chunk(&self, service: &str, window: &str, sequence: u64) -> Result<Chunk> {
        let path = self.chunk_path(service, window, sequence);
        let bytes = self
            .get_object(&path, || format!("chunk {service}/{window}#{sequence}"))
            .await?;
        decode_chunk(&bytes)
    }

    async fn get_index(
        &self,
        service: &str,
        window: &str,
        sequence: u64,
    ) -> Result<ServiceWindowIndex> {
        // The index lives in the chunk object's metadata — no body download.
        let path = self.chunk_path(service, window, sequence);
        self.find_object(&path)
            .await?
            .and_then(|o| Self::object_meta(&o))
            .and_then(|m| m.index)
            .ok_or_else(|| Error::NotFound(format!("index {service}/{window}#{sequence}")))
    }

    async fn list_chunks(&self, service: &str, range: Option<TimeRange>) -> Result<Vec<ChunkRef>> {
        let prefix = format!("{}/chunks/{service}/", self.bucket);
        let events = self
            .sdk
            .object_events(None, None)
            .await
            .map_err(|e| Error::Backend(format!("sia list: {e}")))?;

        let mut refs = Vec::new();
        for event in events {
            if event.deleted {
                continue;
            }
            let Some(object) = event.object else { continue };
            let Some(meta) = Self::object_meta(&object) else {
                continue;
            };
            if !meta.path.starts_with(&prefix) {
                continue;
            }
            let Some(index) = meta.index else { continue };
            if range.is_none_or(|r| overlaps(&index, r)) {
                refs.push(index.chunk);
            }
        }
        refs.sort_by_key(|c| c.sequence);
        Ok(refs)
    }

    async fn read_manifest(&self) -> Result<Manifest> {
        let bytes = self
            .get_object(&self.manifest_path(), || "manifest".to_string())
            .await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn write_manifest(&self, manifest: &Manifest) -> Result<()> {
        let meta = ObjectMeta {
            path: self.manifest_path(),
            index: None,
        };
        self.put_object(&meta, serde_json::to_vec(manifest)?).await
    }
}
