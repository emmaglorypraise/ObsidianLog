//! Filesystem-backed [`StorageBackend`] — the default, Sia-free backend.
//!
//! `LocalBackend` stores objects as files under `<root>/<bucket>`, mirroring the
//! planned on-Sia layout so the rest of the pipeline is identical regardless of
//! backend:
//!
//! ```text
//! <root>/<bucket>/index/<service>/<window>.idx     # ServiceWindowIndex (JSON)
//! <root>/<bucket>/chunks/<service>/<window>.bin     # framed chunk (header + ciphertext)
//! <root>/<bucket>/manifest.json                      # Manifest (JSON)
//! ```
//!
//! Every write is **durable and atomic**: data is written to a temporary file in
//! the destination directory, `fsync`'d, then renamed into place, after which the
//! directory is `fsync`'d to persist the rename. Reads of a missing object return
//! [`Error::NotFound`]. Manifest updates are serialized through an internal lock
//! (see [`LocalBackend::update_manifest`]) so concurrent writers for different
//! services cannot lose each other's changes.
//!
//! Chunk files use a compact frame — a big-endian `u32` header length, the
//! header as JSON, then the raw ciphertext bytes — so ciphertext is stored as-is
//! rather than expanded into a JSON number array.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use obsidianlog_core::backend::{StorageBackend, TimeRange};
use obsidianlog_core::chunk::{Chunk, ChunkHeader, ChunkRef};
use obsidianlog_core::error::{Error, Result};
use obsidianlog_core::index::ServiceWindowIndex;
use obsidianlog_core::manifest::Manifest;

/// Counter giving temp files unique names within a process.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A [`StorageBackend`] backed by a local directory tree under `<root>/<bucket>`.
#[derive(Debug, Clone)]
pub struct LocalBackend {
    root: PathBuf,
    bucket: String,
    /// Serializes manifest read-modify-write so concurrent updates don't clobber.
    manifest_lock: Arc<Mutex<()>>,
}

impl LocalBackend {
    /// Create a backend storing objects under `root/bucket`.
    pub fn new(root: impl Into<PathBuf>, bucket: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            bucket: bucket.into(),
            manifest_lock: Arc::new(Mutex::new(())),
        }
    }

    /// The filesystem root this backend writes under.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The bucket name objects are stored under.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    fn bucket_dir(&self) -> PathBuf {
        self.root.join(&self.bucket)
    }

    fn chunk_path(&self, service: &str, window: &str) -> Result<PathBuf> {
        Ok(self
            .bucket_dir()
            .join("chunks")
            .join(safe(service)?)
            .join(format!("{}.bin", safe(window)?)))
    }

    fn index_path(&self, service: &str, window: &str) -> Result<PathBuf> {
        Ok(self
            .bucket_dir()
            .join("index")
            .join(safe(service)?)
            .join(format!("{}.idx", safe(window)?)))
    }

    fn manifest_path(&self) -> PathBuf {
        self.bucket_dir().join("manifest.json")
    }

    /// Atomically read-modify-write the manifest under the serialization lock.
    ///
    /// Reads the current manifest (or a fresh one if none exists), applies `edit`,
    /// and durably writes it back. Holding the lock across the whole operation is
    /// what makes concurrent updates for different services safe.
    pub async fn update_manifest<F>(&self, edit: F) -> Result<Manifest>
    where
        F: FnOnce(&mut Manifest) + Send,
    {
        let _guard = self.manifest_lock.lock().expect("manifest lock poisoned");
        let mut manifest = match fs::read(self.manifest_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Manifest::new(&self.bucket),
            Err(e) => return Err(Error::Io(e)),
        };
        edit(&mut manifest);
        atomic_write(
            &self.manifest_path(),
            &serde_json::to_vec_pretty(&manifest)?,
        )?;
        Ok(manifest)
    }
}

#[async_trait]
impl StorageBackend for LocalBackend {
    async fn put_chunk(&self, chunk: &Chunk) -> Result<()> {
        let path = self.chunk_path(&chunk.header.service, &chunk.header.time_window)?;
        atomic_write(&path, &encode_chunk(chunk)?)
    }

    async fn get_chunk(&self, service: &str, window: &str) -> Result<Chunk> {
        let path = self.chunk_path(service, window)?;
        let bytes = read_file(&path, || format!("chunk {service}/{window}"))?;
        decode_chunk(&bytes)
    }

    async fn put_index(&self, index: &ServiceWindowIndex) -> Result<()> {
        let path = self.index_path(&index.service, &index.window)?;
        atomic_write(&path, &serde_json::to_vec(index)?)
    }

    async fn get_index(&self, service: &str, window: &str) -> Result<ServiceWindowIndex> {
        let path = self.index_path(service, window)?;
        let bytes = read_file(&path, || format!("index {service}/{window}"))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn list_chunks(&self, service: &str, range: Option<TimeRange>) -> Result<Vec<ChunkRef>> {
        let dir = self.bucket_dir().join("index").join(safe(service)?);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            // No index directory for the service → no chunks.
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::Io(e)),
        };

        let mut refs = Vec::new();
        for entry in entries {
            let path = entry.map_err(Error::Io)?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("idx") {
                continue;
            }
            let index: ServiceWindowIndex = serde_json::from_slice(&fs::read(&path)?)?;
            if range.is_none_or(|r| overlaps(&index, r)) {
                refs.push(index.chunk);
            }
        }
        refs.sort_by_key(|c| c.sequence);
        Ok(refs)
    }

    async fn read_manifest(&self) -> Result<Manifest> {
        let bytes = read_file(&self.manifest_path(), || "manifest".to_string())?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn write_manifest(&self, manifest: &Manifest) -> Result<()> {
        let _guard = self.manifest_lock.lock().expect("manifest lock poisoned");
        atomic_write(&self.manifest_path(), &serde_json::to_vec_pretty(manifest)?)
    }
}

/// Reject path components that could escape the bucket directory.
fn safe(component: &str) -> Result<&str> {
    if component.is_empty()
        || component == ".."
        || component == "."
        || component.contains(['/', '\\'])
    {
        return Err(Error::Backend(format!(
            "unsafe path component: {component:?}"
        )));
    }
    Ok(component)
}

/// Does an index's time span overlap the query range `[start, end]`?
fn overlaps(index: &ServiceWindowIndex, range: TimeRange) -> bool {
    !(index.max_timestamp < range.start || index.min_timestamp > range.end)
}

/// Frame a chunk as `u32(header_len) || header_json || ciphertext`.
fn encode_chunk(chunk: &Chunk) -> Result<Vec<u8>> {
    let header = serde_json::to_vec(&chunk.header)?;
    let mut out = Vec::with_capacity(4 + header.len() + chunk.ciphertext.len());
    out.extend_from_slice(&(header.len() as u32).to_be_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&chunk.ciphertext);
    Ok(out)
}

/// Decode a chunk framed by [`encode_chunk`].
fn decode_chunk(bytes: &[u8]) -> Result<Chunk> {
    let len_bytes: [u8; 4] = bytes
        .get(0..4)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| Error::Serialization("chunk frame too short for header length".into()))?;
    let header_len = u32::from_be_bytes(len_bytes) as usize;
    let header_bytes = bytes
        .get(4..4 + header_len)
        .ok_or_else(|| Error::Serialization("chunk frame truncated in header".into()))?;
    let header: ChunkHeader = serde_json::from_slice(header_bytes)?;
    let ciphertext = bytes[4 + header_len..].to_vec();
    Ok(Chunk { header, ciphertext })
}

/// Read a file, mapping a missing file to [`Error::NotFound`] via `describe`.
fn read_file(path: &Path, describe: impl FnOnce() -> String) -> Result<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(Error::NotFound(describe())),
        Err(e) => Err(Error::Io(e)),
    }
}

/// Durably write `bytes` to `path`: temp file → fsync → rename → fsync dir.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::Backend("destination path has no parent".into()))?;
    fs::create_dir_all(parent)?;

    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".tmp-{}-{}", std::process::id(), n));

    let mut file = fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?; // fsync the contents before the rename
    drop(file);

    fs::rename(&tmp, path)?;

    // fsync the directory so the rename itself survives a crash. Best-effort:
    // some platforms don't support directory fsync.
    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use obsidianlog_core::manifest::ManifestServiceChain;
    use std::collections::BTreeSet;

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(secs, 0).unwrap()
    }

    fn backend() -> (tempfile::TempDir, LocalBackend) {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalBackend::new(dir.path(), "obsidianlog");
        (dir, backend)
    }

    fn chunk(service: &str, window: &str, sequence: u64, ciphertext: &[u8]) -> Chunk {
        Chunk {
            header: ChunkHeader {
                service: service.to_string(),
                time_window: window.to_string(),
                sequence,
                prev_hash: [0u8; 32],
                nonce: [1u8; 12],
                created_at: ts(1_700_000_000),
                record_count: 1,
                uncompressed_len: ciphertext.len() as u64,
            },
            ciphertext: ciphertext.to_vec(),
        }
    }

    fn index(service: &str, window: &str, sequence: u64, min: i64, max: i64) -> ServiceWindowIndex {
        ServiceWindowIndex {
            service: service.to_string(),
            window: window.to_string(),
            min_timestamp: ts(min),
            max_timestamp: ts(max),
            levels: BTreeSet::new(),
            hosts: BTreeSet::new(),
            keywords: BTreeSet::new(),
            chunk: ChunkRef {
                service: service.to_string(),
                window: window.to_string(),
                sequence,
            },
        }
    }

    #[tokio::test]
    async fn chunk_put_get_round_trips() {
        let (_dir, backend) = backend();
        let original = chunk("api", "2026-06-29-15", 0, &[0xDE, 0xAD, 0xBE, 0xEF]);
        backend.put_chunk(&original).await.unwrap();
        let fetched = backend.get_chunk("api", "2026-06-29-15").await.unwrap();
        assert_eq!(fetched, original);
    }

    #[tokio::test]
    async fn index_put_get_round_trips() {
        let (_dir, backend) = backend();
        let original = index("api", "2026-06-29-15", 0, 100, 200);
        backend.put_index(&original).await.unwrap();
        let fetched = backend.get_index("api", "2026-06-29-15").await.unwrap();
        assert_eq!(fetched, original);
    }

    #[tokio::test]
    async fn manifest_read_write_round_trips() {
        let (_dir, backend) = backend();
        let mut manifest = Manifest::new("obsidianlog");
        manifest
            .services
            .insert("api".to_string(), ManifestServiceChain::new("api"));
        backend.write_manifest(&manifest).await.unwrap();
        let fetched = backend.read_manifest().await.unwrap();
        assert_eq!(fetched, manifest);
    }

    #[tokio::test]
    async fn missing_objects_return_not_found() {
        let (_dir, backend) = backend();
        assert!(matches!(
            backend.get_chunk("api", "nope").await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            backend.get_index("api", "nope").await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            backend.read_manifest().await,
            Err(Error::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn list_filters_by_service_and_time_range() {
        let (_dir, backend) = backend();
        // api: two windows; web: one window.
        backend
            .put_index(&index("api", "2026-06-29-15", 0, 100, 200))
            .await
            .unwrap();
        backend
            .put_index(&index("api", "2026-06-29-16", 1, 4000, 4100))
            .await
            .unwrap();
        backend
            .put_index(&index("web", "2026-06-29-15", 0, 100, 200))
            .await
            .unwrap();

        // All of api's chunks, ordered by sequence.
        let all = backend.list_chunks("api", None).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].sequence, 0);
        assert_eq!(all[1].sequence, 1);

        // Only the first window overlaps [50, 300].
        let ranged = backend
            .list_chunks(
                "api",
                Some(TimeRange {
                    start: ts(50),
                    end: ts(300),
                }),
            )
            .await
            .unwrap();
        assert_eq!(ranged.len(), 1);
        assert_eq!(ranged[0].window, "2026-06-29-15");

        // A service with no chunks lists empty.
        assert!(
            backend
                .list_chunks("absent", None)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_manifest_updates_do_not_corrupt() {
        let (_dir, backend) = backend();
        let backend = Arc::new(backend);

        let mut handles = Vec::new();
        for i in 0..16 {
            let backend = Arc::clone(&backend);
            handles.push(tokio::spawn(async move {
                let service = format!("svc-{i}");
                backend
                    .update_manifest(|m| {
                        m.services
                            .insert(service.clone(), ManifestServiceChain::new(&service));
                    })
                    .await
                    .unwrap();
            }));
        }
        for handle in handles {
            handle.await.unwrap();
        }

        // Every concurrent update is present — none were lost to a race.
        let manifest = backend.read_manifest().await.unwrap();
        assert_eq!(manifest.services.len(), 16);
    }
}
