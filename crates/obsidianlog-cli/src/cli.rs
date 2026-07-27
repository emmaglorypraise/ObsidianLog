//! Command-line interface definition.
//!
//! The CLI is declared as a [`clap`] derive tree. Keeping it in the library lets
//! the crate own its own argument surface and lets tests construct invocations
//! directly.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// ObsidianLog: long-term, tamper-evident operational log archival on Sia.
#[derive(Debug, Parser)]
#[command(name = "obsidianlog", version, about, long_about = None)]
pub struct Cli {
    /// Path to the ObsidianLog config file (defaults to the platform config dir).
    #[arg(long, short, global = true, value_name = "FILE")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

/// Top-level subcommands, mirroring the `obsidianlog <verb>` surface.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Interactive setup wizard: generate keys, configure indexd, write config.
    Init(InitArgs),

    /// Run the Vector-compatible HTTP ingest server.
    Serve(ServeArgs),

    /// Query archived logs by time range, service, level, host, or keyword.
    Query(QueryArgs),

    /// Traverse and validate the full hash chain for tamper-evidence.
    Verify(VerifyArgs),
}

/// Arguments for `obsidianlog init`.
#[derive(Debug, Args)]
pub struct InitArgs {
    /// Run non-interactively, accepting defaults where possible.
    #[arg(long)]
    pub non_interactive: bool,

    /// Overwrite an existing configuration if one is present.
    #[arg(long)]
    pub force: bool,
}

/// Arguments for `obsidianlog serve`.
#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Address to bind the HTTP ingest endpoint to (overrides the config file).
    #[arg(long, value_name = "ADDR")]
    pub bind: Option<String>,
}

/// Arguments for `obsidianlog query`.
#[derive(Debug, Args)]
pub struct QueryArgs {
    /// Inclusive lower bound of the time range (RFC 3339).
    #[arg(long, value_name = "RFC3339")]
    pub since: Option<String>,

    /// Exclusive upper bound of the time range (RFC 3339).
    #[arg(long, value_name = "RFC3339")]
    pub until: Option<String>,

    /// Filter by service name.
    #[arg(long, value_name = "NAME")]
    pub service: Option<String>,

    /// Filter by log level (e.g. info, warn, error).
    #[arg(long, value_name = "LEVEL")]
    pub level: Option<String>,

    /// Filter by originating host.
    #[arg(long, value_name = "HOST")]
    pub host: Option<String>,

    /// Free-text keyword to match within log lines.
    #[arg(long, value_name = "TEXT")]
    pub keyword: Option<String>,

    /// Output format for matched records.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,
}

/// Arguments for `obsidianlog verify`.
#[derive(Debug, Args)]
pub struct VerifyArgs {
    /// Restrict verification to a single service's chain.
    #[arg(long, value_name = "NAME")]
    pub service: Option<String>,
}

/// Rendering format for query results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// Human-readable, colorized terminal output.
    Human,
    /// Newline-delimited / structured JSON.
    Json,
    /// Raw decrypted log bytes, suitable for piping to `jq`.
    Raw,
}
