//! Regression test for ADR-0009: cross-service AES-GCM nonce uniqueness.
//!
//! Historically, the nonce discriminator was `SHA-256(service)[..4]` — a
//! 32-bit truncated hash of the (attacker-influenceable) service *name*. Two
//! service names could be found whose discriminators collided (a cheap,
//! deterministic ~2^32 search, since the name is untrusted input copied
//! verbatim from ingested log records), causing two different services to be
//! sealed under the exact same `(key, nonce)` pair — a full break of both
//! confidentiality (the reused keystream leaks the XOR of both plaintexts) and
//! the tamper-evidence guarantee (the reused keystream also exposes the GHASH
//! subkey, enabling forged authentication tags). See ADR-0009.
//!
//! The fix: the discriminator is now a manifest-assigned `service_id`, handed
//! out once per service from an authoritative monotonic counter
//! (`Manifest::next_service_id`) — never derived from the name. This test
//! reuses the exact scenario that was exploitable before the fix (two service
//! names that collide under the old hash-based scheme) and proves it no
//! longer works.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use obsidianlog_core::record::{LogBatch, LogRecord};
use obsidianlog_store::ArchiveEngine;
use obsidianlog_store::backend::{LocalBackend, StorageBackend};
use obsidianlog_store::compress::{DEFAULT_LEVEL, compress, decompress};
use obsidianlog_store::encrypt::{EncryptionKey, TAG_LEN};

/// Reconstruct the pre-fix discriminator, purely to find two names that
/// collided under that scheme — the exact attack surface this test proves is
/// now closed. Not exposed by the crate anymore; recomputed here for the test.
fn old_scheme_discriminator(service: &str) -> [u8; 4] {
    let digest = Sha256::digest(service.as_bytes());
    [digest[0], digest[1], digest[2], digest[3]]
}

/// Deterministic (same pair every run) search for two distinct names that
/// collide under [`old_scheme_discriminator`].
fn find_names_that_collided_under_the_old_scheme() -> (String, String) {
    let mut seen: HashMap<[u8; 4], String> = HashMap::new();
    for i in 0u64..50_000_000 {
        let name = format!("svc-{i}");
        let discriminator = old_scheme_discriminator(&name);
        if let Some(prev) = seen.insert(discriminator, name.clone()) {
            return (prev, name);
        }
    }
    panic!("no 4-byte discriminator collision found — unexpected for a 32-bit space");
}

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

fn xor(a: &[u8], b: &[u8]) -> Vec<u8> {
    a.iter().zip(b).map(|(x, y)| x ^ y).collect()
}

/// Services get distinct, monotonically-assigned ids in first-use order —
/// never derived from (and so never colliding via) the service name.
#[tokio::test]
async fn distinct_services_always_get_distinct_monotonic_ids() {
    let dir = tempfile::tempdir().unwrap();
    let backend = LocalBackend::new(dir.path(), "obsidianlog");
    let engine = ArchiveEngine::new(backend, EncryptionKey::new([0x24; 32]), "obsidianlog");

    for i in 0..50i64 {
        let service = format!("svc-{i}");
        engine
            .ingest_batch(LogBatch(vec![record(&service, i, "hello")]))
            .await
            .unwrap();
    }

    let manifest = engine.backend().read_manifest().await.unwrap();
    let mut ids: Vec<u32> = manifest.services.values().map(|c| c.service_id).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        (0u32..50).collect::<Vec<_>>(),
        "ids must be distinct and assigned 0..n in first-use order"
    );
}

/// The exact scenario that used to break confidentiality: two service names
/// that collide under the old hash-based discriminator. Proves it no longer
/// works — the services get different nonces, and the plaintext-recovery
/// technique that used to fully recover the victim's secret now fails.
#[tokio::test]
async fn a_name_collision_that_used_to_be_exploitable_is_now_harmless() {
    let (victim_service, attacker_service) = find_names_that_collided_under_the_old_scheme();
    assert_eq!(
        old_scheme_discriminator(&victim_service),
        old_scheme_discriminator(&attacker_service),
        "sanity check: these two names really did collide under the old scheme"
    );

    let dir = tempfile::tempdir().unwrap();
    let backend = LocalBackend::new(dir.path(), "obsidianlog");
    // One shared key across all services, exactly as in production.
    let engine = ArchiveEngine::new(backend, EncryptionKey::new([0x7A; 32]), "obsidianlog");

    let secret = "wire-transfer approved acct=1234567890 amount=1000000 memo=PROJECT-OBSIDIAN";
    engine
        .ingest_batch(LogBatch(vec![record(&victim_service, 10, secret)]))
        .await
        .unwrap();

    let attacker_records: Vec<LogRecord> = (0..400)
        .map(|i| {
            record(
                &attacker_service,
                100 + i,
                &format!("attacker filler line {i}"),
            )
        })
        .collect();
    engine
        .ingest_batch(LogBatch(attacker_records.clone()))
        .await
        .unwrap();

    let victim_chunks = engine.service_chunks(&victim_service).await.unwrap();
    let attacker_chunks = engine.service_chunks(&attacker_service).await.unwrap();
    let victim_chunk = &victim_chunks[0];
    let attacker_chunk = &attacker_chunks[0];

    // The fix itself: two services whose *names* collide under the old scheme
    // now get different nonces, because the discriminator no longer comes
    // from the name at all.
    assert_ne!(
        victim_chunk.header.nonce, attacker_chunk.header.nonce,
        "a name collision under the old scheme must no longer produce a shared nonce"
    );

    // Attempt the exact attack that used to fully recover the victim's
    // secret: recover the shared keystream from the attacker's own known
    // plaintext, and use it to "decrypt" the victim's ciphertext. Since the
    // nonces now genuinely differ, the keystreams differ too, so this must
    // NOT recover the real secret.
    let attacker_plaintext = compress(
        &serde_json::to_vec(&attacker_records).unwrap(),
        DEFAULT_LEVEL,
    )
    .unwrap();
    let attacker_body = &attacker_chunk.ciphertext[..attacker_chunk.ciphertext.len() - TAG_LEN];
    let victim_body = &victim_chunk.ciphertext[..victim_chunk.ciphertext.len() - TAG_LEN];
    let usable_len = attacker_body.len().min(attacker_plaintext.len());
    let keystream = xor(
        &attacker_body[..usable_len],
        &attacker_plaintext[..usable_len],
    );
    let attempt_len = keystream.len().min(victim_body.len());
    let attempted = xor(&victim_body[..attempt_len], &keystream[..attempt_len]);

    let recovered_secret = decompress(&attempted)
        .ok()
        .and_then(|plaintext| serde_json::from_slice::<Vec<LogRecord>>(&plaintext).ok())
        .and_then(|records| records.first().cloned())
        .and_then(|r| {
            r.raw
                .get("msg")
                .and_then(|v| v.as_str().map(str::to_string))
        });

    assert_ne!(
        recovered_secret.as_deref(),
        Some(secret),
        "the historical attack must no longer recover the victim's real secret"
    );
}
