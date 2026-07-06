//! Storage backends.
//!
//! The [`StorageBackend`] trait is defined in `obsidianlog-core` and re-exported
//! here. Implementations live in submodules:
//!
//! - [`local`] — [`LocalBackend`], a filesystem-backed store that is the
//!   **default**. The whole pipeline builds, runs, and tests against it with no
//!   Sia node — the mock-first invariant (see ADR-0004).
//! - [`sia`] — [`SiaBackend`], the real Sia integration. Compiled only with the
//!   `sia` feature so the pre-1.0 Sia SDK never enters a default build (ADR-0006).
//!
//! Backends are append-only: written chunks are never modified or deleted
//! post-write, and every write is made durable before it returns `Ok`.
//!
//! Encoding helpers shared by the backends live here: the compact chunk frame
//! (`u32(header_len) || header-json || ciphertext`) and the index/range overlap
//! check.

use obsidianlog_core::backend::TimeRange;
use obsidianlog_core::chunk::{Chunk, ChunkHeader};
use obsidianlog_core::error::{Error, Result};
use obsidianlog_core::index::ServiceWindowIndex;

pub use obsidianlog_core::backend::StorageBackend;

pub mod local;
pub use local::LocalBackend;

#[cfg(feature = "sia")]
pub mod sia;
#[cfg(feature = "sia")]
pub use sia::{SiaBackend, SiaConfig};

/// Does an index's time span overlap the query range `[start, end]`?
pub(crate) fn overlaps(index: &ServiceWindowIndex, range: TimeRange) -> bool {
    !(index.max_timestamp < range.start || index.min_timestamp > range.end)
}

/// Frame a chunk as `u32(header_len) || header_json || ciphertext`, so the
/// ciphertext is stored as-is rather than expanded into a JSON number array.
pub(crate) fn encode_chunk(chunk: &Chunk) -> Result<Vec<u8>> {
    let header = serde_json::to_vec(&chunk.header)?;
    let mut out = Vec::with_capacity(4 + header.len() + chunk.ciphertext.len());
    out.extend_from_slice(&(header.len() as u32).to_be_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&chunk.ciphertext);
    Ok(out)
}

/// Decode a chunk framed by [`encode_chunk`].
pub(crate) fn decode_chunk(bytes: &[u8]) -> Result<Chunk> {
    let len_bytes: [u8; 4] = bytes
        .get(0..4)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| Error::Serialization("chunk frame too short for header length".into()))?;
    let header_len = u32::from_be_bytes(len_bytes) as usize;
    let header_bytes = bytes
        .get(4..4 + header_len)
        .ok_or_else(|| Error::Serialization("chunk frame truncated in header".into()))?;
    let header: ChunkHeader = serde_json::from_slice(header_bytes)?;
    let ciphertext = bytes[4 + header_len..].to_vec();
    Ok(Chunk { header, ciphertext })
}
