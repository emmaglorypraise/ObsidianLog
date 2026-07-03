//! Typed errors for the ingest service.

use thiserror::Error;

/// Convenience alias for results returned across the ingest service.
pub type Result<T> = std::result::Result<T, Error>;

/// Failures the ingest service can surface.
#[derive(Debug, Error)]
pub enum Error {
    /// The server could not bind its listen address or start serving.
    #[error("failed to start ingest server: {0}")]
    Serve(String),

    /// The configuration file could not be read or parsed.
    #[error("invalid config: {0}")]
    Config(String),

    /// An incoming batch could not be parsed as expected JSON log events.
    #[error("invalid ingest payload: {0}")]
    Payload(String),

    /// A storage-pipeline operation failed while archiving a batch.
    #[error(transparent)]
    Store(#[from] obsidianlog_store::Error),

    /// An underlying I/O operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
