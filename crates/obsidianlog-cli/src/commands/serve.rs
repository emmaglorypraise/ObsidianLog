//! `obsidianlog serve` — run the Vector-compatible HTTP ingest server.
//!
//! Thin wrapper: resolves the config and encryption key, builds an
//! [`obsidianlog_ingest::Config`], and hands off to the `obsidianlog-ingest`
//! crate, which owns the HTTP server and the archival pipeline.

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::cli::ServeArgs;
use crate::config::Config;
use crate::keystore;

/// Start the ingest server (blocks until shutdown).
pub fn run(args: ServeArgs, config_path: Option<PathBuf>) -> Result<()> {
    let config = Config::load(config_path.as_deref())?;
    let key = keystore::default_key_store()?
        .load()
        .context("loading the encryption key (run `obsidianlog init` first)")?;

    let ingest_config = obsidianlog_ingest::Config {
        bind: args.bind.unwrap_or(config.serve.bind),
        bucket: config.bucket,
        storage_root: config.local.data_dir,
        window_secs: config.chunking.window_secs,
        encryption_key: key,
    };

    obsidianlog_ingest::serve_blocking(ingest_config).context("running the ingest server")
}
