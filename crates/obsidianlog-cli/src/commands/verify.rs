//! `obsidianlog verify` — hash-chain integrity check.
//!
//! Thin wrapper: resolves the config, then hands off to
//! [`obsidianlog_store::ArchiveEngine::verify_service`] /
//! [`obsidianlog_store::ArchiveEngine::verify_all`], which walk each service's
//! chain from genesis, and prints a clear OK/FAIL summary per service.
//!
//! Verification is defined entirely over encrypted bytes (chunk hashes cover
//! the ciphertext, not the plaintext), so this never decrypts and doesn't need
//! the archive's encryption key or OS-keychain access — a placeholder key is
//! used purely because [`ArchiveEngine`] always carries one.
//!
//! `--format json` prints one JSON object instead, keeping each failure's
//! structured detail (kind, position, sequence) for CI scripts. The exit code
//! is the same in both formats.

use std::path::PathBuf;

use anyhow::{Context, Result};
use obsidianlog_store::ArchiveEngine;
use obsidianlog_store::chain::{ArchiveMismatch, ChainBreakKind, VerifiedChain, VerifyFailure};
use obsidianlog_store::encrypt::EncryptionKey;
use serde_json::{Value, json};

use crate::backend::resolve_backend;
use crate::cli::{VerifyArgs, VerifyFormat};
use crate::config::Config;

/// Verify the hash chain and report any integrity violations.
///
/// Returns `Err` if any service's chain fails verification, so the caller
/// (via `main`'s exit-code mapping) exits non-zero — CI/cron can gate on it.
pub fn run(args: VerifyArgs, config_path: Option<PathBuf>) -> Result<()> {
    let config = Config::load(config_path.as_deref())?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting the verify runtime")?;

    let backend = runtime.block_on(resolve_backend(&config))?;
    let engine = ArchiveEngine::new(
        backend,
        EncryptionKey::new([0u8; 32]),
        config.bucket.clone(),
    );

    let results = match &args.service {
        Some(service) => {
            let result = runtime.block_on(engine.verify_service_detailed(service));
            vec![(service.clone(), result)]
        }
        None => runtime
            .block_on(engine.verify_all_detailed())
            .context("reading the manifest")?,
    };

    let failed = results.iter().any(|(_, result)| result.is_err());
    match args.format {
        VerifyFormat::Human => print_human(&results),
        VerifyFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&json_report(&results))
                .context("serializing the verify report")?
        ),
    }

    if failed {
        anyhow::bail!("hash-chain verification failed");
    }
    Ok(())
}

type ServiceResult = (String, std::result::Result<VerifiedChain, VerifyFailure>);

fn print_human(results: &[ServiceResult]) {
    if results.is_empty() {
        println!("no services archived yet — nothing to verify");
        return;
    }
    for (service, result) in results {
        match result {
            Ok(verified) => println!(
                "OK   {service}: {} chunk(s) verified, chain intact",
                verified.chunks
            ),
            Err(err) => println!("FAIL {service}: {err}"),
        }
    }
}

/// The `--format json` report: `{"ok": bool, "services": [...]}`, one entry
/// per service in the order verified.
fn json_report(results: &[ServiceResult]) -> Value {
    let services: Vec<Value> = results
        .iter()
        .map(|(service, result)| match result {
            Ok(verified) => json!({
                "service": service,
                "ok": true,
                "chunks": verified.chunks,
                "head_hash": verified.head_hash.map(|h| hex(&h)),
            }),
            Err(failure) => json!({
                "service": service,
                "ok": false,
                "error": failure_json(failure),
            }),
        })
        .collect();
    json!({
        "ok": results.iter().all(|(_, result)| result.is_ok()),
        "services": services,
    })
}

/// A failure as `{"kind": ..., "message": ..., <kind-specific fields>}`.
fn failure_json(failure: &VerifyFailure) -> Value {
    let mut value = match failure {
        VerifyFailure::Break(b) => {
            let kind = match b.kind {
                ChainBreakKind::Modified => "modified",
                ChainBreakKind::Reordered => "reordered",
                ChainBreakKind::Missing => "missing",
                ChainBreakKind::HeadMismatch => "head_mismatch",
            };
            json!({ "kind": kind, "position": b.position, "sequence": b.sequence })
        }
        VerifyFailure::Mismatch(m) => match m {
            ArchiveMismatch::Unlisted {
                window,
                sequence,
                count,
            } => json!({
                "kind": "unlisted",
                "window": window,
                "sequence": sequence,
                "count": count,
            }),
            ArchiveMismatch::DuplicateSequence { sequence, count } => json!({
                "kind": "duplicate_sequence",
                "sequence": sequence,
                "count": count,
            }),
            ArchiveMismatch::UnknownService { count } => {
                json!({ "kind": "unknown_service", "count": count })
            }
            ArchiveMismatch::NonceReuse {
                sequence,
                other_service,
                other_sequence,
            } => json!({
                "kind": "nonce_reuse",
                "sequence": sequence,
                "other_service": other_service,
                "other_sequence": other_sequence,
            }),
            _ => json!({ "kind": "mismatch" }),
        },
        _ => json!({ "kind": "error" }),
    };
    value["message"] = Value::String(failure.to_string());
    value
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use obsidianlog_store::chain::ChainBreak;

    #[test]
    fn an_empty_archive_is_ok_with_no_services() {
        assert_eq!(json_report(&[]), json!({ "ok": true, "services": [] }));
    }

    #[test]
    fn an_intact_chain_reports_its_chunks_and_head_hash() {
        let results = vec![(
            "api".to_string(),
            Ok(VerifiedChain {
                chunks: 3,
                head_hash: Some([0xab; 32]),
            }),
        )];
        assert_eq!(
            json_report(&results),
            json!({
                "ok": true,
                "services": [{
                    "service": "api",
                    "ok": true,
                    "chunks": 3,
                    "head_hash": "ab".repeat(32),
                }],
            })
        );
    }

    #[test]
    fn a_break_reports_its_kind_position_and_sequence() {
        let results = vec![(
            "billing".to_string(),
            Err(VerifyFailure::Break(ChainBreak {
                position: 4,
                sequence: 4,
                kind: ChainBreakKind::Missing,
            })),
        )];
        let report = json_report(&results);
        assert_eq!(report["ok"], json!(false));
        let error = &report["services"][0]["error"];
        assert_eq!(error["kind"], "missing");
        assert_eq!(error["position"], 4);
        assert_eq!(error["sequence"], 4);
        assert_eq!(
            error["message"],
            "chain integrity violation: hash-chain break at position 4 (sequence 4): missing chunk"
        );
    }

    #[test]
    fn a_mismatch_reports_its_own_fields() {
        let results = vec![(
            "api".to_string(),
            Err(VerifyFailure::Mismatch(ArchiveMismatch::NonceReuse {
                sequence: 2,
                other_service: "web".to_string(),
                other_sequence: 7,
            })),
        )];
        let error = &json_report(&results)["services"][0]["error"];
        assert_eq!(error["kind"], "nonce_reuse");
        assert_eq!(error["other_service"], "web");
        assert_eq!(error["other_sequence"], 7);
    }
}
