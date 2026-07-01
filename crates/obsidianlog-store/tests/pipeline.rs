//! End-to-end integration tests for the storage pipeline.
//!
//! These drive the public `ArchiveEngine` over a `LocalBackend` — the milestone
//! path: ingest a batch, read it back through decrypt/decompress, and verify the
//! hash chain. They are no longer `#[ignore]`d now that the pipeline is assembled.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

use obsidianlog_core::record::{LogBatch, LogRecord};
use obsidianlog_core::types::ChunkId;
use obsidianlog_store::ArchiveEngine;
use obsidianlog_store::backend::{LocalBackend, StorageBackend};
use obsidianlog_store::chain::{ChainBreakKind, verify_chain};
use obsidianlog_store::encrypt::EncryptionKey;

fn engine(dir: &tempfile::TempDir) -> ArchiveEngine<LocalBackend> {
    let backend = LocalBackend::new(dir.path(), "obsidianlog");
    ArchiveEngine::new(backend, EncryptionKey::new([0x42; 32]), "obsidianlog")
}

fn record(service: &str, id: i64, epoch_secs: i64) -> LogRecord {
    LogRecord {
        raw: serde_json::json!({ "id": id, "msg": "hello world" }),
        timestamp: DateTime::<Utc>::from_timestamp(epoch_secs, 0).unwrap(),
        service: service.to_string(),
        level: Some("info".to_string()),
        host: Some("host-1".to_string()),
        trace_id: None,
    }
}

/// A batch of `count` records for `service`, all within hour `hour`.
fn batch(service: &str, hour: i64, first_id: i64, count: i64) -> LogBatch {
    let base = hour * 3600;
    LogBatch(
        (0..count)
            .map(|i| record(service, first_id + i, base + i))
            .collect(),
    )
}

fn ids(records: &[LogRecord]) -> BTreeSet<i64> {
    records
        .iter()
        .map(|r| r.raw.get("id").and_then(|v| v.as_i64()).unwrap())
        .collect()
}

#[test]
fn chunk_paths_follow_on_sia_layout() {
    // Needs no pipeline logic; locks the on-storage layout.
    let id = ChunkId {
        service: "api".to_string(),
        window: "2026-06-22-15".to_string(),
    };
    assert_eq!(id.chunk_path(), "chunks/api/2026-06-22-15.bin");
    assert_eq!(id.index_path(), "index/api/2026-06-22-15.idx");
}

#[tokio::test]
async fn roundtrip_preserves_log_batch() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir);

    let input = batch("api", 5, 0, 8);
    let expected = ids(&input.0);
    engine.ingest_batch(input).await.unwrap();

    // One window → one chunk; read it back through decrypt + decompress.
    let refs = engine.backend().list_chunks("api", None).await.unwrap();
    assert_eq!(refs.len(), 1);
    let recovered = engine.read_records("api", &refs[0].window).await.unwrap();
    assert_eq!(ids(&recovered), expected, "records must round-trip exactly");
}

#[tokio::test]
async fn tampered_chunk_breaks_the_chain() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir);

    // Three chunks for one service, one per window (sequences 0, 1, 2).
    for hour in 0..3i64 {
        engine
            .ingest_batch(batch("api", hour, hour * 10, 4))
            .await
            .unwrap();
    }

    // Corrupt the middle chunk's ciphertext on disk (flip its last byte).
    let middle = dir.path().join("obsidianlog/chunks/api/1970-01-01-01.bin");
    let mut bytes = std::fs::read(&middle).unwrap();
    *bytes.last_mut().unwrap() ^= 0x01;
    std::fs::write(&middle, &bytes).unwrap();

    // verify_chain detects the modification at the tampered chunk's position.
    let chunks = engine.service_chunks("api").await.unwrap();
    let err = verify_chain(&chunks).expect_err("tampering must break the chain");
    assert_eq!(err.kind, ChainBreakKind::Modified);
    assert_eq!(err.position, 1);
}
