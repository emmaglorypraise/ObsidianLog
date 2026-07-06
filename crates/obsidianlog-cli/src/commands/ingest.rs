//! `obsidianlog ingest` — run the Vector-compatible HTTP ingest server.
//!
//! Thin wrapper: resolves config and hands off to the `obsidianlog-ingest`
//! crate, which owns the HTTP server and the processing pipeline.

use std::path::PathBuf;

use anyhow::Result;

use crate::cli::IngestArgs;

/// Start the ingest server (blocks until shutdown).
///
/// TODO(impl): load config, let `--bind` override `ingest.bind`, then call
/// [`obsidianlog_ingest::serve_blocking`].
pub fn run(args: IngestArgs, config: Option<PathBuf>) -> Result<()> {
    let _ = (args, config);
    anyhow::bail!(
        "obsidianlog ingest is not yet implemented — run the obsidianlog-ingest binary for now (see the README)"
    )
}
