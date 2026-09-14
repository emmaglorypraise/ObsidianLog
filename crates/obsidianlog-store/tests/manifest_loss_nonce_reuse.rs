//! Regression test for the residual gap noted in ADR-0009's Consequences:
//! losing `manifest.json` (or restoring a stale copy) while a service's chunk
//! files remain resets that service's `next_sequence` counter to 0, reusing a
//! nonce already used under the same key for that service. Unlike the
//! cross-service collision ADR-0009 fixed, this needs no attacker and no
//! second service — a lost or stale manifest is enough on its own.
//!
//! `ingest_batch` must refuse to proceed for a service whose manifest entry
//! is missing but whose chunk files already exist on the backend, rather than
//! silently treating it as brand new and restarting the counter at 0.
//!
//! Currently RED: `ArchiveEngine` has no such guard yet — this test documents
//! the gap and will start passing once it's implemented.

use chrono::{DateTime, Utc};

use obsidianlog_core::record::{LogBatch, LogRecord};
use obsidianlog_store::ArchiveEngine;
use obsidianlog_store::backend::LocalBackend;
use obsidianlog_store::encrypt::EncryptionKey;

fn record(service: &str, epoch_secs: i64, msg: &str) -> LogRecord {
    LogRecord {
        raw: serde_json::json!({ "msg": msg }),
        timestamp: DateTime::<Utc>::from_timestamp(epoch_secs, 0).unwrap(),
        service: service.to_string(),
        level: Some("info".to_string()),
        host: Some("host-1".to_string()),
        trace_id: None,
    }
}

#[tokio::test]
async fn ingest_refuses_when_manifest_is_lost_but_chunks_remain() {
    let dir = tempfile::tempdir().unwrap();
    let backend = LocalBackend::new(dir.path(), "obsidianlog");
    let engine = ArchiveEngine::new(backend, EncryptionKey::new([0x42; 32]), "obsidianlog");

    let service = "svc-orders";
    engine
        .ingest_batch(LogBatch(vec![record(service, 10, "first batch")]))
        .await
        .unwrap();

    let chunks_before = engine.service_chunks(service).await.unwrap();
    let original_nonce = chunks_before[0].header.nonce;

    // Simulate manifest loss (or a stale restore) while the chunk files
    // themselves remain on the backend — the trigger ADR-0009 flagged as
    // "arguably the more operationally realistic" of the two gaps.
    let manifest_path = dir.path().join("obsidianlog").join("manifest.json");
    std::fs::remove_file(&manifest_path).unwrap();
    assert!(!manifest_path.exists());

    let result = engine
        .ingest_batch(LogBatch(vec![record(
            service,
            20,
            "second batch, post-loss",
        )]))
        .await;

    assert!(
        result.is_err(),
        "ingest_batch must refuse a service whose manifest entry is missing but whose \
         chunk files already exist on the backend, instead of silently resetting its \
         nonce counter to 0 and reusing a nonce already used under the same key for \
         that service (see ADR-0009's Consequences)"
    );

    // Belt-and-suspenders: even if a future "fix" doesn't hard-refuse but
    // instead resumes correctly, it must never reuse the original nonce.
    if let Ok(chunks_after) = engine.service_chunks(service).await {
        if chunks_after.len() > chunks_before.len() {
            assert_ne!(
                chunks_after[1].header.nonce, original_nonce,
                "must never reuse a (key, nonce) pair for the same service"
            );
        }
    }
}
