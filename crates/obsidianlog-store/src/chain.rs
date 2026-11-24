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
    /// The chain's computed head doesn't match the manifest's recorded head.
    /// Nothing within the chain commits to the last chunk's hash, so tampering
    /// with it can only be caught by checking against the manifest separately.
    HeadMismatch,
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
            ChainBreakKind::HeadMismatch => "head hash mismatch",
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

/// A disagreement between what the backend stores and what the manifest
/// records, found by reconciling the two (ADR-0017).
///
/// Separate from [`ChainBreak`]: a chain can be internally intact while the
/// backend holds chunks the manifest doesn't describe.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ArchiveMismatch {
    /// Chunks are stored that the manifest's chunk list doesn't include, for
    /// example after the manifest was rolled back. `window` and `sequence`
    /// name the first one; `count` is how many there are in total.
    Unlisted {
        /// Time window of the first unlisted chunk.
        window: String,
        /// Sequence number of the first unlisted chunk.
        sequence: u64,
        /// Number of unlisted chunks for the service.
        count: usize,
    },
    /// More than one stored chunk claims the same sequence number: the chain
    /// has forked.
    DuplicateSequence {
        /// The sequence number claimed more than once.
        sequence: u64,
        /// How many stored chunks claim it.
        count: usize,
    },
    /// Chunks are stored for a service the manifest has no record of.
    UnknownService {
        /// Number of chunks stored for the service.
        count: usize,
    },
    /// Two chunks were sealed under the same key with the same nonce.
    NonceReuse {
        /// Sequence number of this service's chunk.
        sequence: u64,
        /// Service of the other chunk (may be the same service).
        other_service: String,
        /// Sequence number of the other chunk.
        other_sequence: u64,
    },
}

impl std::fmt::Display for ArchiveMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unlisted {
                window,
                sequence,
                count,
            } => {
                write!(
                    f,
                    "chunk stored on the backend but missing from the manifest \
                     (window {window}, sequence {sequence})"
                )?;
                if *count > 1 {
                    write!(f, ", and {} more", count - 1)?;
                }
                Ok(())
            }
            Self::DuplicateSequence { sequence, count } => write!(
                f,
                "{count} stored chunks claim sequence {sequence}: the chain has forked"
            ),
            Self::UnknownService { count } => write!(
                f,
                "{count} chunk(s) stored for this service, but the manifest has no record of it"
            ),
            Self::NonceReuse {
                sequence,
                other_service,
                other_sequence,
            } => write!(
                f,
                "chunk at sequence {sequence} reuses the key and nonce of \
                 {other_service} sequence {other_sequence}"
            ),
        }
    }
}

impl std::error::Error for ArchiveMismatch {}

impl From<ArchiveMismatch> for obsidianlog_core::Error {
    fn from(mismatch: ArchiveMismatch) -> Self {
        obsidianlog_core::Error::ChainIntegrity(mismatch.to_string())
    }
}

/// Compare a service's stored chunks (`stored`, as listed by the backend)
/// with the manifest's chunk list (`listed`). Returns the first problem found:
/// a sequence claimed more than once, then chunks the manifest doesn't list.
///
/// Chunks the manifest lists but storage lacks aren't reported here; fetching
/// them during chain verification already reports them as missing.
pub fn reconcile_listing(
    listed: &[obsidianlog_core::chunk::ChunkRef],
    stored: &[obsidianlog_core::chunk::ChunkRef],
) -> Result<(), ArchiveMismatch> {
    let mut per_sequence: BTreeMap<u64, usize> = BTreeMap::new();
    for chunk in stored {
        *per_sequence.entry(chunk.sequence).or_default() += 1;
    }
    if let Some((&sequence, &count)) = per_sequence.iter().find(|&(_, &n)| n > 1) {
        return Err(ArchiveMismatch::DuplicateSequence { sequence, count });
    }

    let unlisted: Vec<_> = stored.iter().filter(|c| !listed.contains(c)).collect();
    if let Some(first) = unlisted.iter().min_by_key(|c| c.sequence) {
        return Err(ArchiveMismatch::Unlisted {
            window: first.window.clone(),
            sequence: first.sequence,
            count: unlisted.len(),
        });
    }
    Ok(())
}

/// The key a chunk was sealed under plus its nonce: two chunks sharing this
/// pair reused an AES-GCM (key, nonce) pair. `key_salt` identifies the
/// per-chunk subkey (ADR-0016); `None` means the master key, used by chunks
/// written before ADR-0016.
pub type NonceKey = (Option<[u8; 32]>, [u8; 12]);

/// Find the first reused (key, nonce) pair among `seen`, given as
/// `(service, sequence, key)` in verification order. Returns the later chunk's
/// service and sequence, plus the earlier chunk's.
pub fn find_nonce_reuse(
    seen: &[(String, u64, NonceKey)],
) -> Option<((String, u64), (String, u64))> {
    let mut first_use: std::collections::HashMap<NonceKey, (&str, u64)> =
        std::collections::HashMap::new();
    for (service, sequence, key) in seen {
        if let Some(&(other_service, other_sequence)) = first_use.get(key) {
            return Some((
                (service.clone(), *sequence),
                (other_service.to_string(), other_sequence),
            ));
        }
        first_use.insert(*key, (service.as_str(), *sequence));
    }
    None
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
                key_salt: None,
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

    fn chunk_ref(window: &str, sequence: u64) -> obsidianlog_core::chunk::ChunkRef {
        obsidianlog_core::chunk::ChunkRef {
            service: "api".to_string(),
            window: window.to_string(),
            sequence,
        }
    }

    #[test]
    fn reconcile_accepts_a_listing_that_matches_the_manifest() {
        let refs = [chunk_ref("w0", 0), chunk_ref("w1", 1)];
        assert_eq!(reconcile_listing(&refs, &refs), Ok(()));
    }

    #[test]
    fn reconcile_reports_the_first_unlisted_chunk_and_the_count() {
        let listed = [chunk_ref("w0", 0)];
        let stored = [chunk_ref("w0", 0), chunk_ref("w2", 2), chunk_ref("w1", 1)];
        assert_eq!(
            reconcile_listing(&listed, &stored),
            Err(ArchiveMismatch::Unlisted {
                window: "w1".to_string(),
                sequence: 1,
                count: 2,
            })
        );
    }

    #[test]
    fn reconcile_reports_a_sequence_claimed_twice_before_unlisted_chunks() {
        let listed = [chunk_ref("w0", 0)];
        let stored = [chunk_ref("w0", 0), chunk_ref("w9", 0)];
        assert_eq!(
            reconcile_listing(&listed, &stored),
            Err(ArchiveMismatch::DuplicateSequence {
                sequence: 0,
                count: 2,
            })
        );
    }

    #[test]
    fn nonce_reuse_is_judged_per_key() {
        let nonce = [7u8; 12];
        // Same nonce under two different per-chunk subkeys: not a reuse.
        let distinct_keys = [
            ("api".to_string(), 0, (Some([1u8; 32]), nonce)),
            ("web".to_string(), 0, (Some([2u8; 32]), nonce)),
        ];
        assert_eq!(find_nonce_reuse(&distinct_keys), None);

        // Same nonce under the master key (chunks from before ADR-0016): reuse.
        let master_key = [
            ("api".to_string(), 0, (None, nonce)),
            ("web".to_string(), 3, (None, nonce)),
        ];
        assert_eq!(
            find_nonce_reuse(&master_key),
            Some((("web".to_string(), 3), ("api".to_string(), 0)))
        );
    }
}
