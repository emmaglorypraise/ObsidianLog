//! `obsidianlog serve` — run the Vector-compatible HTTP ingest server.
//!
//! Thin wrapper: resolves the config, the encryption key, and the backend
//! (local or Sia — see [`resolve_backend`]), builds an [`ArchiveEngine`], and
//! hands off to `obsidianlog-ingest`, which owns the HTTP server.

use std::path::PathBuf;

use anyhow::{Context, Result};
use obsidianlog_store::ArchiveEngine;
use obsidianlog_store::encrypt::EncryptionKey;

use crate::backend::resolve_backend;
use crate::cli::ServeArgs;
use crate::config::Config;
use crate::keystore;

/// Start the ingest server (blocks until shutdown).
pub fn run(args: ServeArgs, config_path: Option<PathBuf>) -> Result<()> {
    let config = Config::load(config_path.as_deref())?;
    let key = EncryptionKey::new(
        keystore::default_encryption_key_store()?
            .0
            .load()
            .context("loading the encryption key (run `obsidianlog init` first)")?,
    );

    let backend = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting the backend-resolution runtime")?
        .block_on(resolve_backend(&config))?;

    let bind = args.bind.unwrap_or(config.serve.bind);
    let engine = ArchiveEngine::new(backend, key, config.bucket)
        .with_window_secs(config.chunking.window_secs);

    obsidianlog_ingest::serve_engine_blocking(&bind, engine).context("running the ingest server")
}
