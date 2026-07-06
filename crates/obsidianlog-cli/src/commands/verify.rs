//! `obsidianlog verify` — hash-chain integrity check.
//!
//! Traverses the manifest's chain head, recomputing `SHA-256(chunk_{n-1})` and
//! comparing it to each chunk's recorded `prev_hash`. Any mismatch, gap, or
//! reorder is reported as a detectable integrity violation. Doubles as a health
//! check that archived evidence is retrievable.

use std::path::PathBuf;

use anyhow::Result;

use crate::cli::VerifyArgs;

/// Verify the hash chain and report any integrity violations.
///
/// TODO(impl): load the manifest via `obsidianlog_store`, walk the chain
/// (optionally scoped to `args.service`), recompute hashes, and report the first
/// break position.
pub fn run(args: VerifyArgs, config: Option<PathBuf>) -> Result<()> {
    let _ = (args, config);
    anyhow::bail!("obsidianlog verify is not yet implemented — see the roadmap")
}
