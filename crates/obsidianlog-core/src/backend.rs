//! The storage backend abstraction.
//!
//! The whole pipeline talks to durable storage only through [`StorageBackend`].
//! Keeping the trait here — in the pure `core` crate, away from every
//! implementation — is what lets the default build and test suite run with **no
//! Sia node**: `obsidianlog-store` ships an in-process `LocalBackend` by default
//! and confines the real Sia integration to a `sia`-feature-gated impl, so the
//! pre-1.0 Sia SDK never leaks into the pure pipeline (see ADR-0004, ADR-0005).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::chunk::{Chunk, ChunkRef};
use crate::error::Result;
use crate::index::ServiceWindowIndex;
use crate::manifest::Manifest;

/// A half-open time range, `[start, end)`, for listing chunks by time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRange {
    /// Inclusive lower bound.
    pub start: DateTime<Utc>,
    /// Exclusive upper bound.
    pub end: DateTime<Utc>,
}

/// Durable, append-only storage for chunks, indexes, and the manifest.
///
/// Implemented by `obsidianlog_store::backend::LocalBackend` (the default) and,
/// with the `sia` feature, `obsidianlog_store::backend::SiaBackend`. A backend
/// may co-locate a chunk and its index (see the Sia backend) or keep them apart
/// (the local backend); the manifest lives at the bucket root.
///
/// ## Durability contract
///
/// Every write method (`put_archive`, `write_manifest`) MUST make the data
/// durable — fsync'd to disk, or acknowledged by the storage network — **before**
/// returning `Ok(())`. Returning `Ok` is a promise that the data will survive a
/// crash. Writes are append-only: a stored chunk is never modified.
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// Durably store a `chunk` together with its metadata `index` as one archive
    /// unit for its `(service, window)`.
    ///
    /// Backends choose the physical layout — separate objects, or a single
    /// content-addressed object carrying the index in its metadata (the Sia
    /// backend) — but **both** parts MUST be durable before returning `Ok`.
    async fn put_archive(&self, chunk: &Chunk, index: &ServiceWindowIndex) -> Result<()>;

    /// Fetch the chunk stored for `(service, window)`.
    async fn get_chunk(&self, service: &str, window: &str) -> Result<Chunk>;

    /// Fetch the metadata index for `(service, window)`.
    async fn get_index(&self, service: &str, window: &str) -> Result<ServiceWindowIndex>;

    /// List chunk references for `service`, optionally restricted to `range`.
    async fn list_chunks(&self, service: &str, range: Option<TimeRange>) -> Result<Vec<ChunkRef>>;

    /// Read the root manifest.
    async fn read_manifest(&self) -> Result<Manifest>;

    /// Durably write the root manifest.
    async fn write_manifest(&self, manifest: &Manifest) -> Result<()>;
}
