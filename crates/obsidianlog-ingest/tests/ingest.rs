//! Integration tests for the ingest service.
//!
//! Each test starts the real server on an ephemeral port, POSTs over HTTP with
//! reqwest (rustls), and — for the happy path — reopens the same store with
//! `LocalBackend` to confirm the batch was archived and the chain verifies.

use std::sync::Arc;

use obsidianlog_ingest::{Config, build_engine, serve_on};
use obsidianlog_store::ArchiveEngine;
use obsidianlog_store::backend::LocalBackend;
use obsidianlog_store::chain::verify_chain;
use obsidianlog_store::encrypt::EncryptionKey;

const KEY: [u8; 32] = [0x42; 32];

/// Start the server on an ephemeral port; returns its base URL.
async fn spawn_server(config: &Config) -> String {
    let engine = Arc::new(build_engine(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { serve_on(listener, engine).await.unwrap() });
    format!("http://{addr}")
}

fn config(root: &std::path::Path) -> Config {
    Config {
        bind: "127.0.0.1:0".to_string(),
        bucket: "obsidianlog".to_string(),
        storage_root: root.to_path_buf(),
        window_secs: 3600,
        encryption_key: KEY,
    }
}

#[tokio::test]
async fn accepts_a_vector_batch_and_archives_it() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let base = spawn_server(&config).await;

    // Health check.
    let client = reqwest::Client::new();
    assert!(
        client
            .get(format!("{base}/health"))
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );

    // A Vector-style JSON array of events.
    let body = serde_json::json!([
        { "timestamp": "2026-07-02T15:00:00Z", "service": "api", "level": "info", "id": 1 },
        { "timestamp": "2026-07-02T15:00:01Z", "service": "api", "level": "error", "id": 2 },
    ]);
    let resp = client
        .post(format!("{base}/ingest"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "batch must be acknowledged");

    // Reopen the same store and confirm the batch was archived + chain verifies.
    let backend = LocalBackend::new(dir.path(), "obsidianlog");
    let engine = ArchiveEngine::new(backend, EncryptionKey::new(KEY), "obsidianlog");

    let chunks = engine.service_chunks("api").await.unwrap();
    assert_eq!(chunks.len(), 1, "one window → one chunk");
    assert!(verify_chain(&chunks).is_ok(), "chain must verify");

    let records = engine
        .read_records("api", &chunks[0].header.time_window)
        .await
        .unwrap();
    assert_eq!(records.len(), 2, "both events must round-trip");
}

#[tokio::test]
async fn rejects_malformed_payload() {
    let dir = tempfile::tempdir().unwrap();
    let base = spawn_server(&config(dir.path())).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/ingest"))
        .header("content-type", "application/json")
        .body("{ this is not valid json")
        .send()
        .await
        .unwrap();

    assert!(
        resp.status().is_client_error(),
        "malformed JSON must be rejected with 4xx, got {}",
        resp.status()
    );
}
