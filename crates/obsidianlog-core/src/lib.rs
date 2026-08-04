//! ObsidianLog core: the shared data model and storage abstraction.
//!
//! This crate owns the vocabulary every other crate is built on. It is **types
//! and serialization only** — no compression, crypto, I/O, or other business
//! logic lives here, so it can be depended on from anywhere without pulling in
//! heavy dependencies.
//!
//! - [`error`] — the canonical [`Error`]/[`Result`].
//! - [`record`] — [`record::LogRecord`] / [`record::LogBatch`], the parsed log
//!   events entering the pipeline.
//! - [`chunk`] — [`chunk::ChunkHeader`], [`chunk::Chunk`], and
//!   [`chunk::ChunkRef`], plus the canonical byte layout used for hashing.
//! - [`index`] — [`index::ServiceWindowIndex`], the lightweight per-window
//!   metadata queried before fetching chunks.
//! - [`manifest`] — [`manifest::Manifest`] / [`manifest::ManifestServiceChain`],
//!   the per-service chain heads and chunk registry.
//! - [`backend`] — the async [`backend::StorageBackend`] trait. It lives here,
//!   **apart from any implementation**, so the pure pipeline crate never
//!   transitively depends on the pre-1.0 Sia SDK (see ADR-0004, ADR-0005).
//! - [`types`] — primitive aliases ([`types::Key`], [`types::ChunkHash`], …).

pub mod backend;
pub mod chunk;
pub mod error;
pub mod index;
pub mod manifest;
pub mod record;
pub mod types;

pub use backend::{StorageBackend, TimeRange};
pub use chunk::{Chunk, ChunkHeader, ChunkRef};
pub use error::{Error, Result};
pub use index::{IndexEntry, ServiceWindowIndex};
pub use manifest::{Manifest, ManifestServiceChain};
pub use record::{LogBatch, LogRecord};

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use serde::Serialize;
    use serde::de::DeserializeOwned;
    use std::collections::BTreeSet;

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(secs, 0).expect("valid timestamp")
    }

    /// Serialize → deserialize → assert the value is unchanged.
    fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) {
        let json = serde_json::to_string(value).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(value, &back, "round-trip changed the value");
    }

    fn sample_header() -> ChunkHeader {
        ChunkHeader {
            service: "api".to_string(),
            time_window: "2026-06-29-15".to_string(),
            sequence: 1,
            prev_hash: [0u8; 32],
            nonce: [0u8; 12],
            created_at: ts(0),
            record_count: 2,
            uncompressed_len: 4,
        }
    }

    #[test]
    fn round_trips_every_type() {
        let record = LogRecord {
            raw: serde_json::json!({ "msg": "hello", "n": 3 }),
            timestamp: ts(1_700_000_000),
            service: "api".to_string(),
            level: Some("error".to_string()),
            host: Some("host-1".to_string()),
            trace_id: None,
        };
        round_trip(&record);
        round_trip(&LogBatch(vec![record.clone()]));

        let header = sample_header();
        round_trip(&header);

        let chunk = Chunk {
            header: header.clone(),
            ciphertext: vec![1, 2, 3, 4],
        };
        round_trip(&chunk);

        let chunk_ref = chunk.chunk_ref();
        round_trip(&chunk_ref);

        let index = ServiceWindowIndex {
            service: "api".to_string(),
            window: "2026-06-29-15".to_string(),
            min_timestamp: ts(10),
            max_timestamp: ts(20),
            levels: BTreeSet::from(["error".to_string(), "info".to_string()]),
            hosts: BTreeSet::from(["host-1".to_string()]),
            keywords: BTreeSet::from(["hello".to_string()]),
            chunk: chunk_ref.clone(),
        };
        round_trip(&index);

        let mut chain = ManifestServiceChain::new("api", 0);
        chain.next_sequence = 2;
        chain.head_hash = [7u8; 32];
        chain.chunks.push(chunk_ref);
        round_trip(&chain);

        let mut manifest = Manifest::new("obsidianlog");
        manifest.services.insert("api".to_string(), chain);
        round_trip(&manifest);

        round_trip(&TimeRange {
            start: ts(0),
            end: ts(100),
        });
    }

    #[test]
    fn chunk_header_canonical_layout_is_stable() {
        let header = sample_header();

        // Built byte-for-byte independently of the implementation, to lock the
        // on-the-wire hashing layout against accidental change.
        let mut expected = Vec::new();
        expected.extend_from_slice(&3u32.to_be_bytes()); // service len
        expected.extend_from_slice(b"api");
        expected.extend_from_slice(&13u32.to_be_bytes()); // time_window len
        expected.extend_from_slice(b"2026-06-29-15");
        expected.extend_from_slice(&1u64.to_be_bytes()); // sequence
        expected.extend_from_slice(&[0u8; 32]); // prev_hash
        expected.extend_from_slice(&[0u8; 12]); // nonce
        expected.extend_from_slice(&0i64.to_be_bytes()); // created_at millis
        expected.extend_from_slice(&2u32.to_be_bytes()); // record_count
        expected.extend_from_slice(&4u64.to_be_bytes()); // uncompressed_len

        assert_eq!(header.canonical_bytes(), expected);
        assert_eq!(expected.len(), 96);
    }

    #[test]
    fn hash_input_is_header_plus_ciphertext_and_excludes_own_hash() {
        let header = sample_header();
        let ciphertext = vec![0xAA, 0xBB, 0xCC];
        let chunk = Chunk {
            header: header.clone(),
            ciphertext: ciphertext.clone(),
        };

        // hash_input == header canonical bytes ++ ciphertext, and nothing else.
        let mut expected = header.canonical_bytes();
        expected.extend_from_slice(&ciphertext);
        assert_eq!(chunk.hash_input(), expected);
        assert_eq!(
            chunk.hash_input().len(),
            header.canonical_bytes().len() + ciphertext.len(),
            "the chunk's own hash must not be part of the hashed bytes"
        );

        // prev_hash IS hashed — that is what chains the chunks together.
        let mut tampered = chunk.clone();
        tampered.header.prev_hash = [9u8; 32];
        assert_ne!(chunk.hash_input(), tampered.hash_input());
    }
}
