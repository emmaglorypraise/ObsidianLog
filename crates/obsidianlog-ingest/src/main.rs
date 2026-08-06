//! Thin standalone binary for the ObsidianLog ingest server.
//!
//! Usage: `obsidianlog-ingest [CONFIG.toml]` — with no argument it uses defaults
//! (binds `127.0.0.1:7080`). The `obsidianlog` CLI's `ingest` subcommand is the
//! usual entry point; this binary exists for running the server directly.
//!
//! The encryption key is never read from the config file — set
//! `OBSIDIANLOG_ENCRYPTION_KEY_FILE` (path to a file holding a 64-character hex
//! key; the convention for a mounted Docker/Kubernetes secret) or
//! `OBSIDIANLOG_ENCRYPTION_KEY` (the hex key itself) before starting.

use std::path::PathBuf;

use obsidianlog_ingest::config::encryption_key_from_env;
use obsidianlog_ingest::{Config, serve};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::args().nth(1).map(PathBuf::from);
    let mut config = Config::load(config_path.as_deref())?;
    config.encryption_key = encryption_key_from_env()?;

    eprintln!(
        "obsidianlog-ingest: listening on http://{} (bucket {:?}, root {})",
        config.bind,
        config.bucket,
        config.storage_root.display()
    );
    serve(config).await?;
    Ok(())
}
