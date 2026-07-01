//! The archive engine: the end-to-end pipeline over any [`StorageBackend`].
//!
//! [`ArchiveEngine::ingest_batch`] takes a batch of parsed records and runs it
//! through the full pipeline — chunk by time window, compress, encrypt, hash-
//! chain, index, and persist — acknowledging only once every write is durable.
//!
//! ## Serialization
//!
//! Each chunk's `prev_hash` and nonce derive from its service's current chain
//! head and sequence number, so two writers for the *same* service must not
//! interleave (that would fork the chain and, fatally, reuse a nonce). The engine
//! therefore holds a **per-service async lock** across the whole
//! read-head → write-chunk → update-manifest sequence. A separate **global lock**
//! is held only around the manifest's read-modify-write, so concurrent writers
//! for *different* services can't clobber the shared `manifest.json`. Different
//! services run in parallel; a single service is strictly serialized (ADR-0003).

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::Mutex as AsyncMutex;

use obsidianlog_core::backend::StorageBackend;
use obsidianlog_core::chunk::{Chunk, ChunkHeader};
use obsidianlog_core::error::{Error, Result};
use obsidianlog_core::manifest::{Manifest, ManifestServiceChain};
use obsidianlog_core::record::{LogBatch, LogRecord};
use obsidianlog_core::types::GENESIS;

use crate::chain::compute_chunk_hash;
use crate::chunking::{Bucket, DEFAULT_WINDOW_SECS, chunk_batch};
use crate::compress::{DEFAULT_LEVEL, compress, decompress};
use crate::encrypt::{EncryptionKey, decrypt_chunk, derive_nonce, encrypt_chunk};
use crate::index::build_index;

/// Ties the storage pipeline together over a [`StorageBackend`].
pub struct ArchiveEngine<B: StorageBackend> {
    backend: B,
    key: EncryptionKey,
    bucket: String,
    window_secs: u64,
    compression_level: i32,
    /// One async lock per service, serializing that service's writers.
    service_locks: StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    /// Serializes the manifest read-modify-write across all services.
    manifest_lock: AsyncMutex<()>,
}

impl<B: StorageBackend> ArchiveEngine<B> {
    /// Create an engine over `backend`, encrypting with `key`, writing under
    /// `bucket`. Uses the default time window and compression level.
    pub fn new(backend: B, key: EncryptionKey, bucket: impl Into<String>) -> Self {
        Self {
            backend,
            key,
            bucket: bucket.into(),
            window_secs: DEFAULT_WINDOW_SECS,
            compression_level: DEFAULT_LEVEL,
            service_locks: StdMutex::new(HashMap::new()),
            manifest_lock: AsyncMutex::new(()),
        }
    }

    /// Override the chunk time-window length (seconds).
    pub fn with_window_secs(mut self, window_secs: u64) -> Self {
        self.window_secs = window_secs;
        self
    }

    /// Override the zstd compression level.
    pub fn with_compression_level(mut self, level: i32) -> Self {
        self.compression_level = level;
        self
    }

    /// Borrow the underlying backend.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Ingest a batch: group into `(service, window)` buckets and archive each,
    /// per-service serialized. Returns only after every write is durable.
    pub async fn ingest_batch(&self, batch: LogBatch) -> Result<()> {
        for bucket in chunk_batch(&batch, self.window_secs) {
            self.ingest_bucket(bucket).await?;
        }
        Ok(())
    }

    async fn ingest_bucket(&self, bucket: Bucket) -> Result<()> {
        // Serialize all writers for this service for the whole sequence below.
        let lock = self.service_lock(&bucket.service);
        let _service_guard = lock.lock().await;

        // (a) Read this service's chain head + next sequence. Safe without the
        // global lock: only this writer (holding the per-service lock) mutates
        // this service's chain.
        let manifest = self.read_manifest_or_default().await?;
        let (prev_hash, sequence) = match manifest.services.get(&bucket.service) {
            Some(chain) => (chain.head_hash, chain.next_sequence),
            None => (GENESIS, 0),
        };

        // (b) Serialize + compress the records.
        let plaintext = serde_json::to_vec(&bucket.records)?;
        let uncompressed_len = plaintext.len() as u64;
        let compressed = compress(&plaintext, self.compression_level)?;

        // (c) Derive the nonce from (service, sequence) and encrypt.
        let nonce = derive_nonce(&bucket.service, sequence);
        let ciphertext = encrypt_chunk(&self.key, nonce, &compressed)?;

        // (d) Assemble the chunk (prev_hash = current head) and hash it.
        let created_at = bucket
            .records
            .iter()
            .map(|r| r.timestamp)
            .max()
            .expect("chunk_batch never yields an empty bucket");
        let chunk = Chunk {
            header: ChunkHeader {
                service: bucket.service.clone(),
                time_window: bucket.window.clone(),
                sequence,
                prev_hash,
                nonce,
                created_at,
                record_count: bucket.records.len() as u32,
                uncompressed_len,
            },
            ciphertext,
        };
        let hash = compute_chunk_hash(&chunk);
        let chunk_ref = chunk.chunk_ref();

        // (e) Build the window's metadata index.
        let index = build_index(chunk_ref.clone(), &bucket.records);

        // (f) Durable writes: chunk + index, then atomically advance the manifest.
        self.backend.put_chunk(&chunk).await?;
        self.backend.put_index(&index).await?;

        let _manifest_guard = self.manifest_lock.lock().await;
        let mut manifest = self.read_manifest_or_default().await?;
        let chain = manifest
            .services
            .entry(bucket.service.clone())
            .or_insert_with(|| ManifestServiceChain::new(&bucket.service));
        chain.head_hash = hash;
        chain.next_sequence += 1;
        chain.chunks.push(chunk_ref);
        self.backend.write_manifest(&manifest).await?;
        Ok(())
    }

    /// Read, decrypt, and decompress the records of one `(service, window)` chunk.
    pub async fn read_records(&self, service: &str, window: &str) -> Result<Vec<LogRecord>> {
        let chunk = self.backend.get_chunk(service, window).await?;
        let compressed = decrypt_chunk(&self.key, chunk.header.nonce, &chunk.ciphertext)?;
        let plaintext = decompress(&compressed)?;
        Ok(serde_json::from_slice(&plaintext)?)
    }

    /// Fetch all of a service's chunks in chain (sequence) order.
    pub async fn service_chunks(&self, service: &str) -> Result<Vec<Chunk>> {
        let refs = self.backend.list_chunks(service, None).await?;
        let mut chunks = Vec::with_capacity(refs.len());
        for chunk_ref in refs {
            chunks.push(self.backend.get_chunk(service, &chunk_ref.window).await?);
        }
        Ok(chunks)
    }

    async fn read_manifest_or_default(&self) -> Result<Manifest> {
        match self.backend.read_manifest().await {
            Ok(manifest) => Ok(manifest),
            Err(Error::NotFound(_)) => Ok(Manifest::new(&self.bucket)),
            Err(e) => Err(e),
        }
    }

    fn service_lock(&self, service: &str) -> Arc<AsyncMutex<()>> {
        let mut locks = self
            .service_locks
            .lock()
            .expect("service-lock map poisoned");
        Arc::clone(
            locks
                .entry(service.to_string())
                .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::LocalBackend;
    use crate::chain::verify_chain;
    use chrono::{DateTime, Utc};
    use std::collections::BTreeSet;

    fn engine(dir: &tempfile::TempDir) -> ArchiveEngine<LocalBackend> {
        let backend = LocalBackend::new(dir.path(), "obsidianlog");
        ArchiveEngine::new(backend, EncryptionKey::new([0x24; 32]), "obsidianlog")
    }

    fn record(service: &str, id: i64, epoch_secs: i64) -> LogRecord {
        LogRecord {
            raw: serde_json::json!({ "id": id, "msg": "event" }),
            timestamp: DateTime::<Utc>::from_timestamp(epoch_secs, 0).unwrap(),
            service: service.to_string(),
            level: Some("info".to_string()),
            host: Some("host-1".to_string()),
            trace_id: None,
        }
    }

    /// A batch of `count` records for `service`, all within hour `hour` so they
    /// land in a single window (a distinct window per batch avoids overwrites).
    fn batch(service: &str, hour: i64, first_id: i64, count: i64) -> LogBatch {
        let base = hour * 3600;
        LogBatch(
            (0..count)
                .map(|i| record(service, first_id + i, base + i))
                .collect(),
        )
    }

    fn ids(records: &[LogRecord]) -> BTreeSet<i64> {
        records
            .iter()
            .map(|r| r.raw.get("id").and_then(|v| v.as_i64()).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn end_to_end_round_trips_two_services() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(&dir);

        // Several batches per service, each in a distinct window.
        let mut expected: HashMap<&str, BTreeSet<i64>> = HashMap::new();
        let mut id = 0;
        for (service, hours) in [("api", [0, 1, 2]), ("web", [0, 1, 2])] {
            for hour in hours {
                let b = batch(service, hour, id, 5);
                expected.entry(service).or_default().extend(ids(&b.0));
                id += 5;
                engine.ingest_batch(b).await.unwrap();
            }
        }

        for service in ["api", "web"] {
            // Read every chunk back, decrypt, and collect the record ids.
            let refs = engine.backend().list_chunks(service, None).await.unwrap();
            assert_eq!(refs.len(), 3, "one chunk per window");
            let mut got = BTreeSet::new();
            for r in &refs {
                got.extend(ids(&engine.read_records(service, &r.window).await.unwrap()));
            }
            assert_eq!(&got, &expected[service], "records must round-trip exactly");

            // The chain verifies.
            let chunks = engine.service_chunks(service).await.unwrap();
            assert!(verify_chain(&chunks).is_ok(), "chain must be intact");

            // Nonces never repeat within the service.
            let nonces: BTreeSet<[u8; 12]> = chunks.iter().map(|c| c.header.nonce).collect();
            assert_eq!(
                nonces.len(),
                chunks.len(),
                "nonces must be unique per service"
            );

            // Sequences are 0..n in order.
            for (i, c) in chunks.iter().enumerate() {
                assert_eq!(c.header.sequence, i as u64);
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_ingest_for_one_service_stays_consistent() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Arc::new(engine(&dir));

        // 12 concurrent batches for one service, each in its own window.
        let mut handles = Vec::new();
        for hour in 0..12i64 {
            let engine = Arc::clone(&engine);
            handles.push(tokio::spawn(async move {
                engine
                    .ingest_batch(batch("api", hour, hour * 100, 3))
                    .await
                    .unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let chunks = engine.service_chunks("api").await.unwrap();
        assert_eq!(chunks.len(), 12);
        // Per-service serialization held: intact chain, unique nonces, all records.
        assert!(verify_chain(&chunks).is_ok());
        let nonces: BTreeSet<[u8; 12]> = chunks.iter().map(|c| c.header.nonce).collect();
        assert_eq!(nonces.len(), 12);
        let total: u32 = chunks.iter().map(|c| c.header.record_count).sum();
        assert_eq!(total, 12 * 3);
    }
}
