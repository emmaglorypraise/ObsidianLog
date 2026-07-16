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
//! interleave (that would fork the chain and, fatally, reuse a nonce). A batch
//! therefore holds the **per-service async lock** of every service it touches for
//! its whole duration. Each window's chunk+index archive is built serially (so
//! the hash chain links in order), but the archives are **uploaded concurrently**
//! (bounded fan-out); the shared `manifest.json` is advanced **once per batch**
//! (ADR-0008): under a separate **global lock** the engine re-reads the manifest,
//! overlays the batch's per-service deltas, and writes it once — so concurrent
//! batches for *different* services can't clobber each other. Different services
//! run in parallel; a single service is strictly serialized (ADR-0003).

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex as StdMutex};

use futures_util::stream::{self, StreamExt};
use tokio::sync::Mutex as AsyncMutex;

use obsidianlog_core::backend::StorageBackend;
use obsidianlog_core::chunk::{Chunk, ChunkHeader, ChunkRef};
use obsidianlog_core::error::{Error, Result};
use obsidianlog_core::index::ServiceWindowIndex;
use obsidianlog_core::manifest::{Manifest, ManifestServiceChain};
use obsidianlog_core::record::{LogBatch, LogRecord};
use obsidianlog_core::types::GENESIS;

use crate::chain::compute_chunk_hash;
use crate::chunking::{DEFAULT_WINDOW_SECS, chunk_batch};
use crate::compress::{DEFAULT_LEVEL, compress, decompress};
use crate::encrypt::{EncryptionKey, decrypt_chunk, derive_nonce, encrypt_chunk};
use crate::index::build_index;

/// Max archives uploaded concurrently per batch (bounds fan-out to the backend).
const MAX_CONCURRENT_UPLOADS: usize = 8;

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

    /// Ingest a batch: group into `(service, window)` buckets, archive each, and
    /// advance the manifest **once** for the whole batch. Per-service serialized;
    /// returns only after every write (including the manifest) is durable.
    pub async fn ingest_batch(&self, batch: LogBatch) -> Result<()> {
        let buckets = chunk_batch(&batch, self.window_secs);
        if buckets.is_empty() {
            return Ok(());
        }

        // Hold each touched service's lock for the whole batch so its chain can't
        // fork and its nonces can't be reused (ADR-0003). Sorted acquisition
        // order avoids deadlock between batches with overlapping services.
        let services: BTreeSet<String> = buckets.iter().map(|b| b.service.clone()).collect();
        let mut _guards = Vec::with_capacity(services.len());
        for service in &services {
            _guards.push(self.service_lock(service).lock_owned().await);
        }

        // Seed each service's chain head + next sequence from the current
        // manifest, then chain forward in memory across the batch's buckets.
        let seed = self.read_manifest_or_default().await?;
        let mut heads: HashMap<String, ([u8; 32], u64)> = HashMap::new();
        let mut new_refs: HashMap<String, Vec<ChunkRef>> = HashMap::new();
        let mut archives: Vec<(Chunk, ServiceWindowIndex)> = Vec::with_capacity(buckets.len());

        for bucket in buckets {
            let (prev_hash, sequence) = match heads.get(&bucket.service) {
                Some(&head) => head,
                None => match seed.services.get(&bucket.service) {
                    Some(chain) => (chain.head_hash, chain.next_sequence),
                    None => (GENESIS, 0),
                },
            };

            // Serialize + compress, derive the nonce, and encrypt.
            let plaintext = serde_json::to_vec(&bucket.records)?;
            let uncompressed_len = plaintext.len() as u64;
            let compressed = compress(&plaintext, self.compression_level)?;
            let nonce = derive_nonce(&bucket.service, sequence);
            let ciphertext = encrypt_chunk(&self.key, nonce, &compressed)?;

            // Assemble + hash the chunk (prev_hash = this service's current head).
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
            let index = build_index(chunk_ref.clone(), &bucket.records);

            heads.insert(bucket.service.clone(), (hash, sequence + 1));
            new_refs.entry(bucket.service).or_default().push(chunk_ref);
            archives.push((chunk, index));
        }

        // Upload the batch's archives concurrently (bounded fan-out). Each
        // archive's chain links are already baked in, so upload order doesn't
        // matter; the manifest is advanced only after every upload is durable.
        // Each future *owns* its (chunk, index), so it borrows nothing from the
        // collection — only `self.backend` — which avoids a higher-ranked
        // lifetime bound the combinator can't satisfy for borrowed items.
        let uploads: Vec<Result<()>> =
            stream::iter(archives.into_iter().map(|(chunk, index)| async move {
                self.backend.put_archive(&chunk, &index).await
            }))
            .buffer_unordered(MAX_CONCURRENT_UPLOADS)
            .collect()
            .await;
        for upload in uploads {
            upload?;
        }

        // One manifest write for the whole batch: under the global lock, re-read
        // (so concurrent batches for other services aren't lost), overlay this
        // batch's per-service deltas, and write once. Ack follows durability.
        let _manifest_guard = self.manifest_lock.lock().await;
        let mut manifest = self.read_manifest_or_default().await?;
        for (service, (head, next_sequence)) in heads {
            let refs = new_refs.remove(&service).unwrap_or_default();
            let chain = manifest
                .services
                .entry(service.clone())
                .or_insert_with(|| ManifestServiceChain::new(&service));
            chain.head_hash = head;
            chain.next_sequence = next_sequence;
            chain.chunks.extend(refs);
        }
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
    use obsidianlog_core::backend::TimeRange;
    use obsidianlog_core::index::ServiceWindowIndex;
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    /// A backend that counts `write_manifest` calls over a real `LocalBackend`.
    struct CountingBackend {
        inner: LocalBackend,
        manifest_writes: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl StorageBackend for CountingBackend {
        async fn put_archive(&self, chunk: &Chunk, index: &ServiceWindowIndex) -> Result<()> {
            self.inner.put_archive(chunk, index).await
        }
        async fn get_chunk(&self, service: &str, window: &str) -> Result<Chunk> {
            self.inner.get_chunk(service, window).await
        }
        async fn get_index(&self, service: &str, window: &str) -> Result<ServiceWindowIndex> {
            self.inner.get_index(service, window).await
        }
        async fn list_chunks(
            &self,
            service: &str,
            range: Option<TimeRange>,
        ) -> Result<Vec<ChunkRef>> {
            self.inner.list_chunks(service, range).await
        }
        async fn read_manifest(&self) -> Result<Manifest> {
            self.inner.read_manifest().await
        }
        async fn write_manifest(&self, manifest: &Manifest) -> Result<()> {
            self.manifest_writes.fetch_add(1, Ordering::SeqCst);
            self.inner.write_manifest(manifest).await
        }
    }

    #[tokio::test]
    async fn multi_window_batch_writes_manifest_once() {
        let dir = tempfile::tempdir().unwrap();
        let backend = CountingBackend {
            inner: LocalBackend::new(dir.path(), "obsidianlog"),
            manifest_writes: AtomicUsize::new(0),
        };
        let engine = ArchiveEngine::new(backend, EncryptionKey::new([0x24; 32]), "obsidianlog");

        // One batch whose records span three windows across two services.
        let mut records = Vec::new();
        let mut id = 0;
        for service in ["api", "web"] {
            for hour in 0..3i64 {
                for _ in 0..4 {
                    records.push(record(service, id, hour * 3600));
                    id += 1;
                }
            }
        }
        engine.ingest_batch(LogBatch(records)).await.unwrap();

        // Coalesced: exactly one manifest write for the whole batch, not per window.
        assert_eq!(
            engine.backend().manifest_writes.load(Ordering::SeqCst),
            1,
            "the manifest must be written once per batch"
        );

        // Everything still round-trips with intact chains.
        for service in ["api", "web"] {
            let refs = engine.backend().list_chunks(service, None).await.unwrap();
            assert_eq!(refs.len(), 3, "one chunk per window");
            let chunks = engine.service_chunks(service).await.unwrap();
            assert!(verify_chain(&chunks).is_ok(), "chain must be intact");
        }
    }
}
