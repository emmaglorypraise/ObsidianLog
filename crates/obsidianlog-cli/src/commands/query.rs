//! `obsidianlog query` — retrieve archived logs.
//!
//! Hits the lightweight metadata index first to identify matching chunks, then
//! fetches and decrypts only those chunks before applying the remaining filters
//! and rendering in the requested output format.

use std::path::PathBuf;

use anyhow::Result;

use crate::cli::QueryArgs;

/// Execute a query and render results to stdout.
///
/// TODO(impl): build an `obsidianlog_store::index::IndexQuery`, prefilter with
/// `might_match`, resolve matching
/// chunks, fetch + decrypt them, apply keyword filtering, and render per
/// `args.format`.
pub fn run(args: QueryArgs, config: Option<PathBuf>) -> Result<()> {
    let _ = (args, config);
    anyhow::bail!("obsidianlog query is not yet implemented — see the roadmap")
}
