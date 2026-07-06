//! `obsidianlog init` — interactive setup wizard.
//!
//! Generates an AES-256 key into the OS keychain (or a `0600` secrets file),
//! prompts for indexd connection details, and writes the config. The target is
//! a clean-machine setup in under 15 minutes.

use std::path::PathBuf;

use anyhow::Result;

use crate::cli::InitArgs;

/// Run the setup wizard.
///
/// TODO(impl): generate the local key via [`crate::keystore`], probe/confirm the
/// indexd endpoint, and write [`crate::config::Config`] (honoring `--force` and
/// `--non-interactive`).
pub fn run(args: InitArgs, config: Option<PathBuf>) -> Result<()> {
    let _ = (args, config);
    anyhow::bail!("obsidianlog init is not yet implemented — see the roadmap")
}
