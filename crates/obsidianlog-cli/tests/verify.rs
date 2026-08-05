//! End-to-end test for `obsidianlog verify`: seeds a fixture archive, runs the
//! actual compiled binary against it, then corrupts a chunk file on disk and
//! confirms the binary detects it, points at the right chunk, and exits
//! non-zero (so CI/cron can gate on it).

use std::path::Path;
use std::process::{Command, Output};

use chrono::{DateTime, Utc};
use obsidianlog_cli::config::{Config, LocalConfig};
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

/// Seed 3 hourly chunks for the "api" service directly through the pipeline
/// (bypassing the CLI's keystore, since the fixture only needs to exist on
/// disk — verify itself never decrypts).
async fn seed(data_dir: &Path) {
    let backend = LocalBackend::new(data_dir, "obsidianlog");
    let engine = ArchiveEngine::new(backend, EncryptionKey::new([0x11; 32]), "obsidianlog");
    for hour in 0..3i64 {
        let records = vec![record("api", hour * 3600, &format!("event {hour}"))];
        engine.ingest_batch(LogBatch(records)).await.unwrap();
    }
}

fn write_config(config_path: &Path, data_dir: &Path) {
    let config = Config {
        local: LocalConfig {
            data_dir: data_dir.to_path_buf(),
        },
        ..Config::default()
    };
    config.save(Some(config_path)).unwrap();
}

fn run_verify(config_path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_obsidianlog"))
        .arg("verify")
        .arg("--config")
        .arg(config_path)
        .output()
        .expect("failed to run `obsidianlog verify`")
}

#[tokio::test]
async fn verify_exits_zero_for_an_intact_archive_and_nonzero_after_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    seed(&data_dir).await;

    let config_path = dir.path().join("config.toml");
    write_config(&config_path, &data_dir);

    // Intact archive: exits zero, reports OK for the service.
    let output = run_verify(&config_path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "expected a zero exit for an intact archive: stdout={stdout} stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("OK") && stdout.contains("api"), "{stdout}");

    // Corrupt the hour-1 chunk file directly on disk (a middle chunk, so the
    // hash-link check — not just the head check — must catch it). Sequence 1:
    // the seed loop ingests hour 0/1/2 as three separate batches for "api",
    // so hour 1 is the second chunk written.
    let chunk_path = data_dir
        .join("obsidianlog")
        .join("chunks")
        .join("api")
        .join("1970-01-01-01-1.bin");
    let mut bytes = std::fs::read(&chunk_path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    std::fs::write(&chunk_path, bytes).unwrap();

    // Corrupted archive: exits non-zero, and points at the right chunk.
    let output = run_verify(&config_path);
    assert!(
        !output.status.success(),
        "expected a non-zero exit after corruption"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("FAIL") && stdout.contains("position 1") && stdout.contains("sequence 1"),
        "must report the failure and point at the tampered chunk: {stdout}"
    );
}

#[tokio::test]
async fn verify_scopes_to_a_single_service_with_the_service_flag() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    seed(&data_dir).await;

    let config_path = dir.path().join("config.toml");
    write_config(&config_path, &data_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_obsidianlog"))
        .arg("verify")
        .arg("--config")
        .arg(&config_path)
        .arg("--service")
        .arg("api")
        .output()
        .expect("failed to run `obsidianlog verify --service api`");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("api"), "{stdout}");

    // A service that was never archived is vacuously OK.
    let output = Command::new(env!("CARGO_BIN_EXE_obsidianlog"))
        .arg("verify")
        .arg("--config")
        .arg(&config_path)
        .arg("--service")
        .arg("nonexistent")
        .output()
        .expect("failed to run `obsidianlog verify --service nonexistent`");
    assert!(output.status.success());
}
