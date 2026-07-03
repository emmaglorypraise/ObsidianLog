//! Thin standalone binary for the ObsidianLog ingest server.
//!
//! Usage: `obsidianlog-ingest [CONFIG.toml]` — with no argument it uses defaults
//! (binds `127.0.0.1:7080`). The `obsidianlog` CLI's `ingest` subcommand is the
//! usual entry point; this binary exists for running the server directly.

use std::path::PathBuf;

use obsidianlog_ingest::{Config, serve};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::args().nth(1).map(PathBuf::from);
    let config = Config::load(config_path.as_deref())?;

    eprintln!(
        "obsidianlog-ingest: listening on http://{} (bucket {:?}, root {})",
        config.bind,
        config.bucket,
        config.storage_root.display()
    );
    serve(config).await?;
    Ok(())
}
