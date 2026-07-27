//! ObsidianLog CLI library.
//!
//! Hosts the command-line surface (`init`, `serve`, `query`, `verify`), config
//! loading, and the OS-keychain key store, wiring them to the
//! [`obsidianlog_store`] and [`obsidianlog_ingest`] crates. The binary in
//! `src/main.rs` is a thin shell over [`run`].
//!
//! # Status
//!
//! The argument surface is complete and dispatches to each command. `serve` is
//! implemented; `init`, `query`, and `verify` are stubs pending the key store,
//! query engine, and chain-verification tooling respectively.

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
        Command::Serve(args) => commands::serve::run(args, cli.config),
        Command::Query(args) => commands::query::run(args, cli.config),
        Command::Verify(args) => commands::verify::run(args, cli.config),
    }
}
