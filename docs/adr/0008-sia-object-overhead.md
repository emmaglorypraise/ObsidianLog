# 0008 — Reducing Sia object overhead (coalesce manifest, fold index)

- Status: Accepted
- Date: 2026-07-16

## Context

The Month 1 end-to-end run against real Sia (ADR-0006) surfaced a cost profile
worth designing around. Ingesting 16 tiny records produced **12 objects**,
provisioned **~503 MB** over **~17 minutes** — about **42 MB and ~87 s per
object**, regardless of payload size. That floor is inherent to Sia: erasure
coding, ~3× host redundancy, and a minimum sector size mean an object costs
roughly the same whether it holds 16 log lines or 16 million.

Two facts follow:

1. **At real volume the floor amortizes.** A 42 MB chunk holds an enormous number
   of compressed log lines, so per-log overhead collapses. The headline 503 MB
   was a small-data artifact, not a per-log cost.
2. **Object *count* is the real lever.** Wall-clock time ≈ objects × ~87 s (each
   object is its own contract + erasure-code + push to ~30 hosts), and each object
   carries the fixed floor. Fewer objects ≈ proportionally less time and size.

Today `ingest_bucket` (one per `(service, window)`) writes **three** objects
(`engine.rs`): `put_chunk`, `put_index`, and `write_manifest`. Two of those are
structurally wasteful on Sia:

- The **index is a separate object** even though it is tiny and always paired 1:1
  with its chunk.
- The **manifest is rewritten on every bucket**. Because Sia objects are
  append-only and content-addressed, every rewrite is a fresh ~42 MB object; old
  manifest versions become garbage, and the read-side scan (`find_object` over
  `object_events`) grows without bound.

## Decision

Two changes, both aimed at cutting object count while preserving the ADR-0003
concurrency invariants (per-service serialization, no nonce reuse, intact chains)
and write-then-ack durability.

### A. Coalesce the manifest to one write per `ingest_batch`

`ingest_batch` will accumulate each service's chain-head/sequence advance in
memory across the batch's buckets and write the manifest **once**, at the end.
The manifest read-modify-write happens under the global manifest lock as a
**re-read → apply this batch's deltas → write**, so two batches touching
different services cannot lose each other's updates (no start-of-batch snapshot).
Chunks and indexes are still made durable before the manifest; acknowledgment
still happens only after the manifest write is durable. A crash mid-batch leaves
orphaned (un-referenced) chunks, which is the existing write-then-ack failure
mode — Vector retries.

### B. Fold the index into the chunk's object

The separate `put_chunk` + `put_index` pair becomes a **single archive write**
carrying both the chunk and its index. Each backend chooses storage:

- **LocalBackend** keeps its current on-disk layout (the change is internal).
- **SiaBackend** stores the index in the object's **`metadata`** blob — which
  `object_events` returns *without* downloading the object body — and the
  ciphertext in the body. Prefiltering (`might_match`) then reads metadata only,
  as before, but each `(service, window)` is **one object instead of two**.

`get_index` / `list_chunks` on Sia read the index from object metadata rather
than from separate index objects.

### Explicitly out of scope (follow-ups)

- **Parallelizing** independent uploads (chunk uploads across services/windows)
  to cut wall-clock time further — a separate change, tracked next.
- **Larger default windows for Sia** — configuration/guidance, not code.
- **Recording object IDs in the manifest** for direct `Sdk::object` fetch, to
  kill the O(objects) read scan (already flagged in ADR-0006).

## Consequences

- **~⅓ fewer objects for the milestone workload.** Folding the index removes one
  object per bucket (3→2), and per-batch manifest coalescing removes the
  per-bucket manifest churn — roughly **~335 MB / ~12 min** for the same 16-record
  test, and no accumulating manifest garbage as data grows.
- **The `StorageBackend` contract changes** (combined chunk+index write; index
  read from metadata on Sia). This is a breaking *internal* API change; core, the
  engine, `LocalBackend`, and `SiaBackend` update together.
- **Concurrency must be re-verified.** The existing round-trip, concurrent-ingest,
  and tamper-detection tests must stay green; add a test asserting a multi-window
  batch writes the manifest exactly once.
- **Metadata size is an implementation risk.** The index must fit within Sia's
  object-metadata limit. Verify during implementation; if an index can exceed the
  limit, fall back to a separate index object for that window rather than failing
  the write.
- Supersede or amend this ADR if the follow-ups (object-ID manifest, parallelism)
  materially change the data model.
