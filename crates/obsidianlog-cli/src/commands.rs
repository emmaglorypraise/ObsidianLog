//! Command handlers backing each CLI subcommand.
//!
//! Each submodule owns one verb. Handlers validate inputs and delegate into the
//! `obsidianlog-store` / `obsidianlog-ingest` crates; the dispatch shape is in
//! place so the real logic has a clear home.

pub mod init;
pub mod query;
pub mod serve;
pub mod verify;
