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
//! arbitrary `metadata: Vec<u8>` blob. We store each object's bucket-relative
//! path (`<bucket>/chunks/<service>/<window>.bin`, etc.) in that metadata, and
//! resolve reads by scanning the indexer's object list (`object_events`) for a
//! matching path. This keeps `LocalBackend` and `SiaBackend` interchangeable
//! behind [`StorageBackend`].
//!
//! Reads are therefore `O(objects)` per lookup — acceptable for the pre-1.0
//! integration; a future optimization is to record each chunk's object id in the
//! manifest and fetch it directly with `Sdk::object`. Objects are also
//! encrypted by the SDK, on top of our own AES-256-GCM (defense in depth).

use std::io::Cursor;

use async_trait::async_trait;
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

    fn chunk_path(&self, service: &str, window: &str) -> String {
        self.object_path("chunks", service, window, "bin")
    }

    fn index_path(&self, service: &str, window: &str) -> String {
        self.object_path("index", service, window, "idx")
    }

    fn manifest_path(&self) -> String {
        format!("{}/manifest.json", self.bucket)
    }

    /// Upload `bytes` as an object whose metadata records `path`, then pin it.
    async fn put_object(&self, path: String, bytes: Vec<u8>) -> Result<()> {
        let mut object = Object::default();
        object.metadata = path.into_bytes();
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

    /// Find the newest non-deleted object whose metadata equals `path`.
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
            if object.metadata.as_slice() != path.as_bytes() {
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
    async fn put_chunk(&self, chunk: &Chunk) -> Result<()> {
        let path = self.chunk_path(&chunk.header.service, &chunk.header.time_window);
        self.put_object(path, encode_chunk(chunk)?).await
    }

    async fn get_chunk(&self, service: &str, window: &str) -> Result<Chunk> {
        let path = self.chunk_path(service, window);
        let bytes = self
            .get_object(&path, || format!("chunk {service}/{window}"))
            .await?;
        decode_chunk(&bytes)
    }

    async fn put_index(&self, index: &ServiceWindowIndex) -> Result<()> {
        let path = self.index_path(&index.service, &index.window);
        self.put_object(path, serde_json::to_vec(index)?).await
    }

    async fn get_index(&self, service: &str, window: &str) -> Result<ServiceWindowIndex> {
        let path = self.index_path(service, window);
        let bytes = self
            .get_object(&path, || format!("index {service}/{window}"))
            .await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn list_chunks(&self, service: &str, range: Option<TimeRange>) -> Result<Vec<ChunkRef>> {
        let prefix = format!("{}/index/{service}/", self.bucket);
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
            let Ok(meta) = std::str::from_utf8(&object.metadata) else {
                continue;
            };
            if !meta.starts_with(&prefix) {
                continue;
            }
            let bytes = self.read_object(&object).await?;
            let index: ServiceWindowIndex = serde_json::from_slice(&bytes)?;
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
        self.put_object(self.manifest_path(), serde_json::to_vec(manifest)?)
            .await
    }
}
