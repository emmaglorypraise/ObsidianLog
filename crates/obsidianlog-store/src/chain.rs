//! Per-service hash chaining for tamper-evidence.
//!
//! Each encrypted chunk is identified by the SHA-256 of its canonical bytes
//! (header — including `prev_hash` — followed by ciphertext; see
//! [`obsidianlog_core::chunk::Chunk::hash_input`]). Chunks are linked into a
//! chain **per service**: the first chunk's `prev_hash` is all-zero
//! ([`GENESIS`]), and every later chunk's `prev_hash` is the hash of its
//! predecessor. Any modification, reorder, or deletion breaks the chain at a
//! detectable position. See ADR-0003.

use std::collections::BTreeMap;

use obsidianlog_core::chunk::Chunk;
use obsidianlog_core::types::GENESIS;
use sha2::{Digest, Sha256};

/// Compute the SHA-256 hash of a chunk over its canonical bytes.
///
/// The bytes hashed are exactly [`Chunk::hash_input`] — the header (which
/// includes `prev_hash`) followed by the ciphertext. The chunk's own hash is
/// stored externally (in the next chunk's `prev_hash` and the manifest) and is
/// never part of the input.
pub fn compute_chunk_hash(chunk: &Chunk) -> [u8; 32] {
    let digest = Sha256::digest(chunk.hash_input());
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// Builds per-service chains by assigning each chunk its sequence number and
/// `prev_hash`, in append order.
///
/// Writes within a service must be serialized — each chunk's `prev_hash` depends
/// on the previous chunk's hash — so a single `ChainBuilder` drives one service's
/// appends. Different services are independent (see ADR-0003).
#[derive(Debug, Default)]
pub struct ChainBuilder {
    chains: BTreeMap<String, ChainState>,
}

#[derive(Debug, Clone, Copy)]
struct ChainState {
    head_hash: [u8; 32],
    next_sequence: u64,
}

impl ChainBuilder {
    /// A builder with no chains yet (every service starts at genesis).
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a service's chain from persisted state (e.g. the manifest head), so
    /// appends resume after the existing chunks instead of restarting at genesis.
    pub fn resume(&mut self, service: impl Into<String>, head_hash: [u8; 32], next_sequence: u64) {
        self.chains.insert(
            service.into(),
            ChainState {
                head_hash,
                next_sequence,
            },
        );
    }

    /// Link `chunk` onto its service's chain: set its `sequence` and `prev_hash`,
    /// then advance the head. Returns the chunk's hash (the new chain head).
    pub fn append(&mut self, chunk: &mut Chunk) -> [u8; 32] {
        let state = self
            .chains
            .entry(chunk.header.service.clone())
            .or_insert(ChainState {
                head_hash: GENESIS,
                next_sequence: 0,
            });

        chunk.header.prev_hash = state.head_hash;
        chunk.header.sequence = state.next_sequence;

        let hash = compute_chunk_hash(chunk);
        state.head_hash = hash;
        state.next_sequence += 1;
        hash
    }

    /// Current head hash of `service`'s chain, if any chunks were appended.
    pub fn head(&self, service: &str) -> Option<[u8; 32]> {
        self.chains.get(service).map(|s| s.head_hash)
    }
}

/// The kind of integrity violation [`verify_chain`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainBreakKind {
    /// A chunk's content changed: it no longer matches the hash its successor
    /// committed to.
    Modified,
    /// The chunks are present but out of order.
    Reordered,
    /// A chunk is missing — the sequence numbers have a gap.
    Missing,
}

/// A detected break in a hash chain: where it is and what kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainBreak {
    /// Index, within the verified slice, of the offending chunk.
    pub position: usize,
    /// Sequence number recorded at that position.
    pub sequence: u64,
    /// What kind of violation it is.
    pub kind: ChainBreakKind,
}

impl std::fmt::Display for ChainBreak {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.kind {
            ChainBreakKind::Modified => "modified chunk",
            ChainBreakKind::Reordered => "reordered chunks",
            ChainBreakKind::Missing => "missing chunk",
        };
        write!(
            f,
            "hash-chain break at position {} (sequence {}): {kind}",
            self.position, self.sequence
        )
    }
}

impl std::error::Error for ChainBreak {}

impl From<ChainBreak> for obsidianlog_core::Error {
    fn from(break_: ChainBreak) -> Self {
        obsidianlog_core::Error::ChainIntegrity(break_.to_string())
    }
}

/// Verify that `chunks` form an intact chain for a single service.
///
/// The slice must be one service's chunks in chain order, starting at the
/// service's first chunk. Detection:
///
/// - **Missing** — the sequence numbers have a gap (a chunk was removed).
/// - **Reordered** — all sequence numbers are present but out of order.
/// - **Modified** — sequences are contiguous and ordered, but a `prev_hash` does
///   not match the recomputed hash of its predecessor (a chunk's content was
///   altered). Reported at the position of the altered chunk.
///
/// Note: tampering with the *last* chunk cannot be detected from the slice alone
/// (nothing commits to its hash); the head chunk is checked against the
/// manifest's recorded head separately.
pub fn verify_chain(chunks: &[Chunk]) -> Result<(), ChainBreak> {
    if chunks.is_empty() {
        return Ok(());
    }

    let base = chunks[0].header.sequence;

    // 1. Sequence integrity: must be base, base+1, base+2, … in order.
    for (i, chunk) in chunks.iter().enumerate() {
        if chunk.header.sequence != base + i as u64 {
            // Distinguish a reorder (a permutation of the full range) from a gap.
            let mut seqs: Vec<u64> = chunks.iter().map(|c| c.header.sequence).collect();
            seqs.sort_unstable();
            let is_permutation = seqs.iter().enumerate().all(|(j, &s)| s == base + j as u64);
            let kind = if is_permutation {
                ChainBreakKind::Reordered
            } else {
                ChainBreakKind::Missing
            };
            return Err(ChainBreak {
                position: i,
                sequence: chunk.header.sequence,
                kind,
            });
        }
    }

    // 2. Hash linkage: each prev_hash must equal the recomputed predecessor hash.
    let mut expected_prev = GENESIS;
    for (i, chunk) in chunks.iter().enumerate() {
        if chunk.header.prev_hash != expected_prev {
            // The predecessor's content changed (its hash no longer matches what
            // this chunk recorded). Report the altered predecessor.
            let position = i.saturating_sub(1);
            return Err(ChainBreak {
                position,
                sequence: chunks[position].header.sequence,
                kind: ChainBreakKind::Modified,
            });
        }
        expected_prev = compute_chunk_hash(chunk);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use obsidianlog_core::chunk::{Chunk, ChunkHeader};

    fn chunk(service: &str, ciphertext: &[u8]) -> Chunk {
        Chunk {
            header: ChunkHeader {
                service: service.to_string(),
                time_window: "2026-06-29-15".to_string(),
                sequence: 0,          // assigned by the builder
                prev_hash: [0u8; 32], // assigned by the builder
                nonce: [0u8; 12],
                created_at: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
                record_count: 1,
                uncompressed_len: ciphertext.len() as u64,
            },
            ciphertext: ciphertext.to_vec(),
        }
    }

    fn built_chain(n: u8) -> Vec<Chunk> {
        let mut builder = ChainBuilder::new();
        let mut chunks: Vec<Chunk> = (0..n).map(|i| chunk("api", &[i; 16])).collect();
        for c in chunks.iter_mut() {
            builder.append(c);
        }
        chunks
    }

    #[test]
    fn builder_links_genesis_and_sequences() {
        let chunks = built_chain(3);
        assert_eq!(chunks[0].header.prev_hash, GENESIS);
        assert_eq!(chunks[0].header.sequence, 0);
        assert_eq!(chunks[1].header.sequence, 1);
        assert_eq!(chunks[1].header.prev_hash, compute_chunk_hash(&chunks[0]));
        assert_eq!(chunks[2].header.prev_hash, compute_chunk_hash(&chunks[1]));
    }

    #[test]
    fn valid_chain_verifies() {
        assert!(verify_chain(&built_chain(5)).is_ok());
    }

    #[test]
    fn modified_middle_chunk_is_detected_at_its_position() {
        let mut chunks = built_chain(5);
        chunks[2].ciphertext[0] ^= 0xFF; // tamper a middle chunk's ciphertext

        let err = verify_chain(&chunks).expect_err("tampering must be caught");
        assert_eq!(err.kind, ChainBreakKind::Modified);
        assert_eq!(err.position, 2, "should point at the altered chunk");
    }

    #[test]
    fn removed_chunk_is_detected_as_missing() {
        let mut chunks = built_chain(5);
        chunks.remove(2); // drop a chunk → sequence gap

        let err = verify_chain(&chunks).expect_err("a gap must be caught");
        assert_eq!(err.kind, ChainBreakKind::Missing);
    }

    #[test]
    fn reordered_chunks_are_detected() {
        let mut chunks = built_chain(5);
        chunks.swap(1, 2); // present but out of order

        let err = verify_chain(&chunks).expect_err("a reorder must be caught");
        assert_eq!(err.kind, ChainBreakKind::Reordered);
        assert_eq!(err.position, 1);
    }
}
