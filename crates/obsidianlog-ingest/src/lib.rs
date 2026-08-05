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
use obsidianlog_store::backend::{LocalBackend, StorageBackend};

use crate::server::{SharedEngine, build_router};

/// Build an [`ArchiveEngine`] over a filesystem [`LocalBackend`] from `config`.
pub fn build_engine(config: &Config) -> ArchiveEngine<LocalBackend> {
    let backend = LocalBackend::new(&config.storage_root, &config.bucket);
    ArchiveEngine::new(
        backend,
        config.encryption_key.clone(),
        config.bucket.clone(),
    )
    .with_window_secs(config.window_secs)
}

/// Serve the ingest router on `listener` until it is closed (no signal handling).
///
/// Useful for tests that bind an ephemeral port themselves; production callers
/// use [`serve`] (or [`serve_engine`]/[`serve_engine_blocking`] when the
/// backend was chosen by the caller rather than always `LocalBackend`).
pub async fn serve_on<B: StorageBackend + Send + Sync + 'static>(
    listener: tokio::net::TcpListener,
    engine: SharedEngine<B>,
) -> Result<()> {
    axum::serve(listener, build_router(engine))
        .await
        .map_err(|e| Error::Serve(e.to_string()))
}

/// Run the ingest server on `bind` until shutdown (Ctrl-C), over an
/// already-built `engine`. The backend-agnostic counterpart to [`serve`], for
/// callers (the CLI) that pick the backend themselves — e.g. Local vs Sia,
/// from config — rather than always using [`build_engine`]'s `LocalBackend`.
pub async fn serve_engine<B: StorageBackend + Send + Sync + 'static>(
    bind: &str,
    engine: ArchiveEngine<B>,
) -> Result<()> {
    let engine = Arc::new(engine);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| Error::Serve(format!("bind {bind}: {e}")))?;

    axum::serve(listener, build_router(engine))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| Error::Serve(e.to_string()))
}

/// Synchronous entry point for callers without an async runtime: builds a
/// multi-thread Tokio runtime and blocks on [`serve_engine`].
pub fn serve_engine_blocking<B: StorageBackend + Send + Sync + 'static>(
    bind: &str,
    engine: ArchiveEngine<B>,
) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(serve_engine(bind, engine))
}

/// Run the ingest server until shutdown (Ctrl-C), binding `config.bind` and
/// archiving to a filesystem [`LocalBackend`] built from `config`. The
/// simple, local-only entry point — used by the standalone
/// `obsidianlog-ingest` binary, which has no reason to support Sia directly.
pub async fn serve(config: Config) -> Result<()> {
    if config.encryption_key.is_placeholder() {
        eprintln!(
            "warning: ingest is running with an all-zero encryption key; \
             set a real key before archiving production data"
        );
    }
    serve_engine(&config.bind, build_engine(&config)).await
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
