//! Shared domain types: the value vocabulary that crosses crate boundaries.
//!
//! Everything here is small, pure, and dependency-light so it can be referenced
//! from the pipeline, the backends, the ingest service, and the CLI alike.

/// Size of an AES-256 key, in bytes.
pub const KEY_LEN: usize = 32;

/// Size of the AES-GCM nonce, in bytes (96 bits).
pub const NONCE_LEN: usize = 12;

/// A 256-bit symmetric key. Owned by the user; never persisted by the library.
pub type Key = [u8; KEY_LEN];

/// A 32-byte SHA-256 digest identifying a chunk in a hash chain.
pub type ChunkHash = [u8; 32];

/// The genesis link: the `prev_hash` value for the first chunk in a chain.
///
/// Chains are maintained **per service** (see ADR-0003), so each service's first
/// chunk uses this value as its `prev_hash`.
pub const GENESIS: ChunkHash = [0u8; 32];
