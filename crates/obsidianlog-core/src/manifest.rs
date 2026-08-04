//! Root manifest: per-service chain heads and the chunk registry.
//!
//! `manifest.json` lives at the bucket root and tracks, for each service, its
//! unique id, hash-chain head, the next sequence number to assign, and the
//! ordered list of its chunks. Chains are per service (see ADR-0003), so the
//! manifest holds one [`ManifestServiceChain`] per service.
//!
//! ## Service ids and nonce uniqueness (ADR-0009)
//!
//! Each service is assigned a `service_id` once, at first use, from
//! [`Manifest::next_service_id`] — a manifest-authoritative monotonic counter,
//! never derived from the (attacker-influenceable) service *name*. The AES-GCM
//! nonce discriminator is this id, not a hash of the name, so cross-service
//! nonce uniqueness is unconditional rather than resting on a 32-bit hash never
//! colliding. See ADR-0009 and `obsidianlog_store::encrypt::derive_nonce`.
//!
//! This is a breaking change from schema version 1: a v1 manifest has no
//! `service_id`/`next_service_id` fields and fails to deserialize against this
//! version deliberately (see [`MANIFEST_VERSION`]) — silently defaulting a
//! missing `service_id` would reintroduce the exact collision this fixes,
//! since every pre-existing chain would default to the same value.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::chunk::ChunkRef;

/// Schema version written into newly created [`Manifest`]s.
///
/// Bumped from 1 to 2 by ADR-0009 (per-service nonce discriminator ids): a
/// v1 manifest is intentionally incompatible, not silently reinterpreted.
pub const MANIFEST_VERSION: u32 = 2;

/// One service's append-only hash chain: its id, head, next sequence, and
/// chunks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestServiceChain {
    /// Service this chain belongs to.
    pub service: String,
    /// Unique id assigned to this service at first use (see the module docs).
    /// Used as the AES-GCM nonce discriminator instead of a hash of `service`.
    pub service_id: u32,
    /// SHA-256 of the chain's head (most recent) chunk; `[0u8; 32]` until the
    /// first chunk is written.
    pub head_hash: [u8; 32],
    /// Sequence number to assign to the next chunk appended to this chain.
    pub next_sequence: u64,
    /// All chunks in this service's chain, in write order.
    pub chunks: Vec<ChunkRef>,
}

impl ManifestServiceChain {
    /// A fresh, empty chain for `service` (genesis head, sequence 0), assigned
    /// `service_id` (from [`Manifest::next_service_id`] — never compute this
    /// from the service name).
    pub fn new(service: impl Into<String>, service_id: u32) -> Self {
        Self {
            service: service.into(),
            service_id,
            head_hash: [0u8; 32],
            next_sequence: 0,
            chunks: Vec::new(),
        }
    }
}

/// The root manifest: per-service chains plus the bucket and schema version.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Storage bucket / namespace these chains live under.
    pub bucket: String,
    /// Per-service chains, keyed by service name.
    pub services: BTreeMap<String, ManifestServiceChain>,
    /// The next `service_id` to assign to a not-yet-seen service. Monotonic;
    /// never reused, even if a service is (hypothetically) ever removed.
    pub next_service_id: u32,
    /// Manifest schema version (see [`MANIFEST_VERSION`]).
    pub version: u32,
}

impl Manifest {
    /// A new, empty manifest for `bucket` at the current schema version.
    pub fn new(bucket: impl Into<String>) -> Self {
        Self {
            bucket: bucket.into(),
            services: BTreeMap::new(),
            next_service_id: 0,
            version: MANIFEST_VERSION,
        }
    }
}
