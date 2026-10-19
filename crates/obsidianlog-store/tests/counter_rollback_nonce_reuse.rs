//! Regression tests for ADR-0016: a rolled-back sequence counter must not
//! reuse an AES-GCM `(key, nonce)` pair.
//!
//! The nonce is derived from the manifest's per-service `next_sequence`
//! counter, which is advanced only *after* a batch's chunks are uploaded. Any
//! time that persisted counter falls behind what was actually used, the next
//! chunk repeats a sequence — and therefore a nonce. Reported by the Month 3
//! reviewer; each scenario below reproduced a real reuse before ADR-0016:
//!
//! 1. A manifest write fails after the chunk upload succeeded, and the next
//!    batch (Vector's retry, or just the next logs) reuses the sequence.
//! 2. The whole data directory is restored from an older backup.
//! 3. Only `manifest.json` is restored from an older backup while the chunk
//!    files stay — ADR-0013's guard doesn't catch this, since the service still
//!    has a manifest entry, just a stale one.
//!
//! The sequence (and so the nonce) still repeats in these scenarios; ADR-0016
//! makes that harmless by sealing each chunk under its own random-salt subkey.
//! The tests therefore assert on the *effective* pair, `(key_salt, nonce)`.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};

use obsidianlog_core::backend::TimeRange;
use obsidianlog_core::chunk::{Chunk, ChunkRef};
use obsidianlog_core::error::{Error, Result};
use obsidianlog_core::index::ServiceWindowIndex;
use obsidianlog_core::manifest::Manifest;
use obsidianlog_core::record::{LogBatch, LogRecord};
use obsidianlog_store::ArchiveEngine;
use obsidianlog_store::backend::{LocalBackend, StorageBackend};
use obsidianlog_store::encrypt::EncryptionKey;
use obsidianlog_store::index::IndexQuery;

const SERVICE: &str = "svc-orders";
const HOUR: i64 = 3600;

fn record(epoch_secs: i64, msg: &str) -> LogRecord {
    LogRecord {
        raw: serde_json::json!({ "msg": msg }),
        timestamp: DateTime::<Utc>::from_timestamp(epoch_secs, 0).unwrap(),
        service: SERVICE.to_string(),
        level: Some("info".to_string()),
        host: Some("host-1".to_string()),
        trace_id: None,
    }
}

fn batch(epoch_secs: i64, msg: &str) -> LogBatch {
    LogBatch(vec![record(epoch_secs, msg)])
}

/// A [`LocalBackend`] whose next `write_manifest` can be made to fail.
struct FlakyManifestBackend {
    inner: LocalBackend,
    fail_next_manifest_write: AtomicBool,
}

#[async_trait::async_trait]
impl StorageBackend for FlakyManifestBackend {
    async fn put_archive(&self, chunk: &Chunk, index: &ServiceWindowIndex) -> Result<()> {
        self.inner.put_archive(chunk, index).await
    }
    async fn get_chunk(&self, service: &str, window: &str, sequence: u64) -> Result<Chunk> {
        self.inner.get_chunk(service, window, sequence).await
    }
    async fn get_index(
        &self,
        service: &str,
        window: &str,
        sequence: u64,
    ) -> Result<ServiceWindowIndex> {
        self.inner.get_index(service, window, sequence).await
    }
    async fn list_chunks(&self, service: &str, range: Option<TimeRange>) -> Result<Vec<ChunkRef>> {
        self.inner.list_chunks(service, range).await
    }
    async fn read_manifest(&self) -> Result<Manifest> {
        self.inner.read_manifest().await
    }
    async fn write_manifest(&self, manifest: &Manifest) -> Result<()> {
        if self.fail_next_manifest_write.swap(false, Ordering::SeqCst) {
            return Err(Error::Backend("injected manifest write failure".into()));
        }
        self.inner.write_manifest(manifest).await
    }
}

fn local_engine(dir: &Path) -> ArchiveEngine<LocalBackend> {
    ArchiveEngine::new(
        LocalBackend::new(dir, "obsidianlog"),
        EncryptionKey::new([0x42; 32]),
        "obsidianlog",
    )
}

/// Every chunk physically present on the backend for [`SERVICE`] — including
/// ones the manifest no longer references.
async fn chunks_on_backend<B: StorageBackend>(backend: &B) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    for r in backend.list_chunks(SERVICE, None).await.unwrap() {
        chunks.push(
            backend
                .get_chunk(SERVICE, &r.window, r.sequence)
                .await
                .unwrap(),
        );
    }
    chunks
}

/// Panics if any two chunks share an effective `(key_salt, nonce)` pair.
fn assert_no_effective_reuse(chunks: &[Chunk]) {
    let mut seen = HashSet::new();
    for chunk in chunks {
        assert!(
            chunk.header.key_salt.is_some(),
            "every newly written chunk must carry a per-chunk key salt (ADR-0016)"
        );
        assert!(
            seen.insert((chunk.header.key_salt, chunk.header.nonce)),
            "(key, nonce) pair reused: sequence {} nonce {:?}",
            chunk.header.sequence,
            chunk.header.nonce
        );
    }
}

async fn all_messages<B: StorageBackend>(engine: &ArchiveEngine<B>) -> Vec<String> {
    let query = IndexQuery {
        service: Some(SERVICE.to_string()),
        ..IndexQuery::default()
    };
    let mut msgs: Vec<String> = engine
        .query(&query)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.raw["msg"].as_str().unwrap().to_string())
        .collect();
    msgs.sort();
    msgs
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

#[tokio::test]
async fn failed_manifest_write_then_next_batch_does_not_reuse_a_key_nonce_pair() {
    let dir = tempfile::tempdir().unwrap();
    let engine = ArchiveEngine::new(
        FlakyManifestBackend {
            inner: LocalBackend::new(dir.path(), "obsidianlog"),
            fail_next_manifest_write: AtomicBool::new(false),
        },
        EncryptionKey::new([0x42; 32]),
        "obsidianlog",
    );

    engine.ingest_batch(batch(0, "first")).await.unwrap();

    // The chunk upload succeeds but the manifest write fails, so the request
    // errors and the counter is never advanced past this chunk's sequence.
    engine
        .backend()
        .fail_next_manifest_write
        .store(true, Ordering::SeqCst);
    assert!(
        engine
            .ingest_batch(batch(HOUR, "orphaned by failed write"))
            .await
            .is_err()
    );

    // The next batch reuses that sequence.
    engine
        .ingest_batch(batch(2 * HOUR, "next batch"))
        .await
        .unwrap();

    let chunks = chunks_on_backend(engine.backend()).await;
    assert_eq!(
        chunks.len(),
        3,
        "the orphaned chunk is still on the backend"
    );
    assert_eq!(
        chunks[1].header.nonce, chunks[2].header.nonce,
        "precondition: this scenario repeats the sequence-derived nonce"
    );
    assert_no_effective_reuse(&chunks);
}

#[tokio::test]
async fn restoring_the_whole_data_dir_from_backup_does_not_reuse_a_key_nonce_pair() {
    let dir = tempfile::tempdir().unwrap();
    let backup = tempfile::tempdir().unwrap();
    let bucket = dir.path().join("obsidianlog");
    let engine = local_engine(dir.path());

    engine.ingest_batch(batch(0, "first")).await.unwrap();
    copy_dir(&bucket, &backup.path().join("obsidianlog"));

    engine
        .ingest_batch(batch(HOUR, "written after the backup"))
        .await
        .unwrap();
    // Its ciphertext may live on elsewhere (another backup, a Sia host), even
    // though the restore below removes it from this directory.
    let lost = chunks_on_backend(engine.backend()).await.pop().unwrap();

    std::fs::remove_dir_all(&bucket).unwrap();
    copy_dir(&backup.path().join("obsidianlog"), &bucket);
    engine
        .ingest_batch(batch(2 * HOUR, "after the restore"))
        .await
        .unwrap();

    let mut chunks = chunks_on_backend(engine.backend()).await;
    let resumed = chunks.last().unwrap().clone();
    assert_eq!(
        lost.header.nonce, resumed.header.nonce,
        "precondition: this scenario repeats the sequence-derived nonce"
    );
    chunks.push(lost);
    assert_no_effective_reuse(&chunks);
    assert_eq!(all_messages(&engine).await, ["after the restore", "first"]);
}

#[tokio::test]
async fn restoring_only_a_stale_manifest_does_not_reuse_a_key_nonce_pair() {
    let dir = tempfile::tempdir().unwrap();
    let manifest_path = dir.path().join("obsidianlog").join("manifest.json");
    let engine = local_engine(dir.path());

    engine.ingest_batch(batch(0, "first")).await.unwrap();
    let stale_manifest = std::fs::read(&manifest_path).unwrap();

    engine
        .ingest_batch(batch(HOUR, "written after the backup"))
        .await
        .unwrap();
    std::fs::write(&manifest_path, stale_manifest).unwrap();

    engine
        .ingest_batch(batch(2 * HOUR, "after the restore"))
        .await
        .unwrap();

    let chunks = chunks_on_backend(engine.backend()).await;
    assert_eq!(chunks.len(), 3);
    assert_eq!(
        chunks[1].header.nonce, chunks[2].header.nonce,
        "precondition: this scenario repeats the sequence-derived nonce"
    );
    assert_no_effective_reuse(&chunks);
}
