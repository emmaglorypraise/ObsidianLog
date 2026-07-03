//! HTTP routing and handlers for the ingest endpoint.
//!
//! `POST /ingest` accepts what Vector's HTTP sink sends with
//! `encoding.codec = "json"` — a JSON array of log objects — and also tolerates
//! newline-delimited JSON. It parses the batch and archives it via the
//! [`ArchiveEngine`], acknowledging with `200` **only after** a durable write.
//! Failures return `5xx` so Vector (which owns buffering, backpressure, and
//! retry) retries. Malformed payloads return `400`.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use chrono::Utc;

use obsidianlog_core::record::LogBatch;
use obsidianlog_store::ArchiveEngine;
use obsidianlog_store::backend::LocalBackend;
use obsidianlog_store::parse::parse_value;

use crate::error::{Error, Result};

/// The path Vector's HTTP sink posts batches to.
pub const INGEST_PATH: &str = "/ingest";

/// Shared engine state; one filesystem-backed engine serves all requests.
pub type SharedEngine = Arc<ArchiveEngine<LocalBackend>>;

/// Build the ingest router: `POST /ingest` and `GET /health`.
pub fn build_router(engine: SharedEngine) -> Router {
    Router::new()
        .route(INGEST_PATH, post(ingest))
        .route("/health", get(health))
        .with_state(engine)
}

async fn health() -> StatusCode {
    StatusCode::OK
}

/// Archive a batch, then acknowledge (write-then-ack).
async fn ingest(State(engine): State<SharedEngine>, body: Bytes) -> Response {
    let batch = match parse_batch(&body) {
        Ok(batch) => batch,
        // Malformed envelope → 400 (Vector should not retry a bad payload).
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };

    match engine.ingest_batch(batch).await {
        Ok(()) => StatusCode::OK.into_response(),
        // Durable write failed → 5xx so Vector retries.
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("archive failed: {e}"),
        )
            .into_response(),
    }
}

/// Parse a Vector HTTP-sink body into a [`LogBatch`].
///
/// Accepts a JSON array of objects (the `encoding.codec = "json"` shape) or
/// newline-delimited JSON. Individual events with missing or odd *fields* are
/// tolerated (see [`parse_value`]); a body that isn't valid JSON at all is
/// rejected. The ingest time seeds the fallback timestamp for events without one.
fn parse_batch(body: &[u8]) -> Result<LogBatch> {
    let ingest_time = Utc::now();

    // Preferred shape: a JSON array of events.
    if let Ok(values) = serde_json::from_slice::<Vec<serde_json::Value>>(body) {
        let records = values
            .into_iter()
            .map(|value| parse_value(value, ingest_time).record)
            .collect();
        return Ok(LogBatch(records));
    }

    // Otherwise newline-delimited JSON (also covers a single JSON object).
    let text = std::str::from_utf8(body).map_err(|_| Error::Payload("body is not UTF-8".into()))?;
    let mut records = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|e| Error::Payload(format!("invalid JSON: {e}")))?;
        records.push(parse_value(value, ingest_time).record);
    }

    if records.is_empty() {
        return Err(Error::Payload("empty or non-JSON payload".into()));
    }
    Ok(LogBatch(records))
}
