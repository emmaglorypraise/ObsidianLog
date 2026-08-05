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

use std::path::PathBuf;

use anyhow::{Context, Result};
use obsidianlog_store::ArchiveEngine;
use obsidianlog_store::encrypt::EncryptionKey;

use crate::backend::resolve_backend;
use crate::cli::VerifyArgs;
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
            let result = runtime.block_on(engine.verify_service(service));
            vec![(service.clone(), result)]
        }
        None => runtime
            .block_on(engine.verify_all())
            .context("reading the manifest")?,
    };

    if results.is_empty() {
        println!("no services archived yet — nothing to verify");
        return Ok(());
    }

    let mut failed = false;
    for (service, result) in &results {
        match result {
            Ok(count) => println!("OK   {service}: {count} chunk(s) verified, chain intact"),
            Err(err) => {
                failed = true;
                println!("FAIL {service}: {err}");
            }
        }
    }

    if failed {
        anyhow::bail!("hash-chain verification failed");
    }
    Ok(())
}
