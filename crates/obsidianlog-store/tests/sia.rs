//! End-to-end integration test for the Sia backend.
//!
//! Compiled only with `--features sia`. It **skips cleanly** unless
//! `OBSIDIANLOG_INDEXD_URL` (the indexer base URL) and `OBSIDIANLOG_APP_KEY`
//! (a 64-hex approved `AppKey`) are set, so CI — which has no indexd — is
//! unaffected. When they are set, it runs the same end-to-end assertions as the
//! `LocalBackend` milestone test, now against Sia: ingest → read back → verify.

#![cfg(feature = "sia")]

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

use obsidianlog_core::record::{LogBatch, LogRecord};
use obsidianlog_store::ArchiveEngine;
use obsidianlog_store::backend::{SiaBackend, SiaConfig, StorageBackend};
use obsidianlog_store::chain::verify_chain;
use obsidianlog_store::encrypt::EncryptionKey;

fn record(service: &str, id: i64, epoch_secs: i64) -> LogRecord {
    LogRecord {
        raw: serde_json::json!({ "id": id, "msg": "event" }),
        timestamp: DateTime::<Utc>::from_timestamp(epoch_secs, 0).unwrap(),
        service: service.to_string(),
        level: Some("info".to_string()),
        host: Some("host-1".to_string()),
        trace_id: None,
    }
}

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

/// Decode a 64-hex string into a 32-byte AppKey.
fn decode_key(hex: &str) -> [u8; 32] {
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex"))
        .collect();
    bytes
        .try_into()
        .expect("OBSIDIANLOG_APP_KEY must be 32 bytes (64 hex chars)")
}

#[tokio::test]
async fn sia_end_to_end_when_indexd_available() {
    let Ok(indexer_url) = std::env::var("OBSIDIANLOG_INDEXD_URL") else {
        eprintln!("skipping sia test: OBSIDIANLOG_INDEXD_URL not set");
        return;
    };
    let app_key = decode_key(
        &std::env::var("OBSIDIANLOG_APP_KEY")
            .expect("OBSIDIANLOG_APP_KEY (64 hex) is required when OBSIDIANLOG_INDEXD_URL is set"),
    );

    // A per-process bucket keeps repeated runs from colliding.
    let bucket = format!("obsidianlog-test-{}", std::process::id());
    let backend = SiaBackend::connect(SiaConfig {
        indexer_url,
        bucket: bucket.clone(),
        app_key,
    })
    .await
    .expect("connect to indexd");
    let engine = ArchiveEngine::new(backend, EncryptionKey::new([0x24; 32]), bucket);

    // Ingest a few batches across two services, each in a distinct window.
    let mut expected: std::collections::HashMap<&str, BTreeSet<i64>> =
        std::collections::HashMap::new();
    let mut id = 0;
    for (service, hours) in [("api", [0, 1]), ("web", [0, 1])] {
        for hour in hours {
            let b = batch(service, hour, id, 4);
            expected.entry(service).or_default().extend(ids(&b.0));
            id += 4;
            engine.ingest_batch(b).await.expect("ingest");
        }
    }

    for service in ["api", "web"] {
        let refs = engine
            .backend()
            .list_chunks(service, None)
            .await
            .expect("list");
        assert_eq!(refs.len(), 2);

        let mut got = BTreeSet::new();
        for r in &refs {
            got.extend(ids(&engine
                .read_records(service, &r.window)
                .await
                .expect("read")));
        }
        assert_eq!(&got, &expected[service], "records must round-trip via Sia");

        let chunks = engine.service_chunks(service).await.expect("chunks");
        assert!(verify_chain(&chunks).is_ok(), "chain must verify");
    }
}
