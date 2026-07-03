//! ObsidianLog HTTP ingest service.
//!
//! A lightweight local HTTP server that receives batched JSON log events from
//! Vector's built-in HTTP sink at `POST /ingest`, then drives each batch through
//! the [`obsidianlog_store`] pipeline (parse → chunk → compress → encrypt →
//! hash-chain → index → persist) via an [`ArchiveEngine`] over a filesystem
//! [`LocalBackend`].
//!
//! **Write-then-ack:** a batch is acknowledged with `200` only after it is
//! durably written; failures return `5xx`. Vector owns buffering, backpressure,
//! and retry, so this service stays small — accept, archive, acknowledge.

pub mod config;
pub mod error;
pub mod server;

pub use config::Config;
pub use error::{Error, Result};

use std::sync::Arc;

use obsidianlog_store::ArchiveEngine;
use obsidianlog_store::backend::LocalBackend;
use obsidianlog_store::encrypt::EncryptionKey;

use crate::server::{SharedEngine, build_router};

/// Build an [`ArchiveEngine`] over a filesystem [`LocalBackend`] from `config`.
pub fn build_engine(config: &Config) -> ArchiveEngine<LocalBackend> {
    let backend = LocalBackend::new(&config.storage_root, &config.bucket);
    ArchiveEngine::new(
        backend,
        EncryptionKey::new(config.encryption_key),
        config.bucket.clone(),
    )
    .with_window_secs(config.window_secs)
}

/// Serve the ingest router on `listener` until it is closed (no signal handling).
///
/// Useful for tests that bind an ephemeral port themselves; production callers
/// use [`serve`].
pub async fn serve_on(listener: tokio::net::TcpListener, engine: SharedEngine) -> Result<()> {
    axum::serve(listener, build_router(engine))
        .await
        .map_err(|e| Error::Serve(e.to_string()))
}

/// Run the ingest server until shutdown (Ctrl-C), binding `config.bind`.
pub async fn serve(config: Config) -> Result<()> {
    if config.encryption_key == [0u8; 32] {
        eprintln!(
            "warning: ingest is running with an all-zero encryption key; \
             set a real key before archiving production data"
        );
    }

    let engine = Arc::new(build_engine(&config));
    let listener = tokio::net::TcpListener::bind(&config.bind)
        .await
        .map_err(|e| Error::Serve(format!("bind {}: {e}", config.bind)))?;

    axum::serve(listener, build_router(engine))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| Error::Serve(e.to_string()))
}

/// Synchronous entry point for callers without an async runtime (e.g. the CLI):
/// builds a multi-thread Tokio runtime and blocks on [`serve`].
pub fn serve_blocking(config: Config) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(serve(config))
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
