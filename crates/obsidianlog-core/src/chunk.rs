//! Chunk references, headers, and the encrypted chunk body.
//!
//! A chunk is the unit written to storage: one compressed-then-encrypted blob of
//! log records for a `(service, time_window)`, plus a header carrying the
//! per-service hash-chain link, the AES-GCM nonce, and the salt its per-chunk
//! subkey was derived from.
//!
//! ## Canonical hashing layout
//!
//! [`Chunk::hash_input`] returns the exact bytes that are SHA-256'd to produce a
//! chunk's hash. Those bytes are the header (which **includes** `prev_hash`)
//! followed by the ciphertext. The chunk's *own* hash is never part of this
//! input — it is computed from these bytes and stored separately (in the next
//! chunk's `prev_hash` and in the manifest). Because `prev_hash` is hashed,
//! tampering with the chain is detectable. The layout is fixed and big-endian;
//! see [`ChunkHeader::canonical_bytes`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A reference to a stored chunk: its service, time window, and chain sequence.
///
/// This is the `(service, window, sequence)` reference used by the index and the
/// manifest to point at a chunk.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkRef {
    /// Service the chunk belongs to.
    pub service: String,
    /// Time-window label, `YYYY-MM-DD-HH`.
    pub window: String,
    /// Position of the chunk in its service's chain.
    pub sequence: u64,
}

/// Header for an encrypted chunk: metadata plus the per-service chain link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkHeader {
    /// Service the chunk belongs to (the chain partition key).
    pub service: String,
    /// Time-window label, `YYYY-MM-DD-HH`.
    pub time_window: String,
    /// Monotonic position of this chunk in its service's chain.
    pub sequence: u64,
    /// SHA-256 of the previous chunk in this service's chain (`[0u8; 32]` for the
    /// genesis chunk).
    pub prev_hash: [u8; 32],
    /// AES-256-GCM nonce used for this chunk's ciphertext.
    pub nonce: [u8; 12],
    /// Random HKDF salt this chunk's AES subkey was derived from (ADR-0016).
    /// `None` for chunks written before ADR-0016, which were sealed directly
    /// under the master key; omitted from the stored header in that case, so
    /// those chunks still decode and hash exactly as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_salt: Option<[u8; 32]>,
    /// When the chunk was sealed (UTC).
    pub created_at: DateTime<Utc>,
    /// Number of log records in the chunk.
    pub record_count: u32,
    /// Uncompressed byte length of the records, before compression/encryption.
    pub uncompressed_len: u64,
}

impl ChunkHeader {
    /// The canonical, deterministic byte encoding of this header, used for
    /// hashing.
    ///
    /// Fixed layout; all integers big-endian; strings are length-prefixed with a
    /// big-endian `u32`:
    ///
    /// | field | encoding |
    /// | --- | --- |
    /// | `service` | `u32` length + UTF-8 bytes |
    /// | `time_window` | `u32` length + UTF-8 bytes |
    /// | `sequence` | `u64` |
    /// | `prev_hash` | 32 bytes |
    /// | `nonce` | 12 bytes |
    /// | `created_at` | `i64` milliseconds since the Unix epoch |
    /// | `record_count` | `u32` |
    /// | `uncompressed_len` | `u64` |
    /// | `key_salt` | only if present: `0x01` + 32 bytes |
    ///
    /// `key_salt` is appended only when present, so headers written before
    /// ADR-0016 hash to exactly the same bytes as they always did.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        push_str(&mut out, &self.service);
        push_str(&mut out, &self.time_window);
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.extend_from_slice(&self.prev_hash);
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&self.created_at.timestamp_millis().to_be_bytes());
        out.extend_from_slice(&self.record_count.to_be_bytes());
        out.extend_from_slice(&self.uncompressed_len.to_be_bytes());
        if let Some(salt) = &self.key_salt {
            out.push(0x01);
            out.extend_from_slice(salt);
        }
        out
    }
}

/// An encrypted chunk: its [`ChunkHeader`] and the AES-256-GCM ciphertext.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    /// Header carrying the chain link, nonce, and metadata.
    pub header: ChunkHeader,
    /// The compressed-then-encrypted log records (`nonce` lives in the header).
    pub ciphertext: Vec<u8>,
}

impl Chunk {
    /// The exact bytes hashed to produce this chunk's SHA-256 hash:
    /// `header.canonical_bytes()` followed by `ciphertext`.
    ///
    /// The chunk's own hash is intentionally **not** included — it is derived
    /// from these bytes and stored in the next chunk's `prev_hash` and in the
    /// manifest. `prev_hash` *is* included, which is what links the chain.
    pub fn hash_input(&self) -> Vec<u8> {
        let mut out = self.header.canonical_bytes();
        out.extend_from_slice(&self.ciphertext);
        out
    }

    /// The `(service, window, sequence)` reference identifying this chunk.
    pub fn chunk_ref(&self) -> ChunkRef {
        ChunkRef {
            service: self.header.service.clone(),
            window: self.header.time_window.clone(),
            sequence: self.header.sequence,
        }
    }
}

/// Append a length-prefixed (`u32` big-endian) UTF-8 string to `out`.
fn push_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}
