//! ObsidianLog CLI library.
//!
//! Hosts the command-line surface (`init`, `ingest`, `query`, `verify`), config
//! loading, and the OS-keychain key store, wiring them to the
//! [`obsidianlog_store`] and [`obsidianlog_ingest`] crates. The binary in
//! `src/main.rs` is a thin shell over [`run`].
//!
//! # Status
//!
//! The argument surface is complete and dispatches to each command; the command
//! implementations (`init`, `ingest`, `query`, `verify`) are in progress.

pub mod cli;
pub mod commands;
pub mod config;
pub mod keystore;

pub use cli::{Cli, Command};
pub use config::Config;

use anyhow::Result;

/// Dispatch a parsed [`Cli`] invocation to the matching command handler.
///
/// This is the single entry point the binary calls into. The global `--config`
/// path is threaded through to commands that read persisted state.
pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Init(args) => commands::init::run(args, cli.config),
        Command::Ingest(args) => commands::ingest::run(args, cli.config),
        Command::Query(args) => commands::query::run(args, cli.config),
        Command::Verify(args) => commands::verify::run(args, cli.config),
    }
}
