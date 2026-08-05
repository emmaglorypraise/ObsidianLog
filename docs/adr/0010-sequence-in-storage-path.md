# 0010 — Chunk sequence as part of the storage path

- Status: Accepted
- Date: 2026-08-05

## Context

`StorageBackend::get_chunk`/`get_index` and both backends' path derivation
(`LocalBackend::chunk_path`/`index_path`, `SiaBackend::chunk_path`) were keyed
by `(service, window)` alone: `chunks/<service>/<window>.bin`,
`index/<service>/<window>.idx` (ADR-0004, ADR-0006).

This was wrong whenever a single time window received more than one
`ingest_batch` call — an ordinary occurrence, not an edge case (a busy
service, a restart mid-window, Vector re-flushing after a network blip all
produce it). The manifest correctly appended a second `ChunkRef` (sequence 1)
for the window, but the second `put_archive` **physically overwrote** the
first chunk and index file on `LocalBackend`, since both batches computed the
same path. `SiaBackend` doesn't overwrite (content-addressed storage,
metadata-tagged by path — ADR-0006), but `find_object` resolves a path to only
the most-recently-updated matching object, so the effect for a reader is the
same: the first batch's data becomes permanently unreachable through the
`StorageBackend` API, while its object keeps costing storage indefinitely
(never deleted, just orphaned).

Confirmed locally: `obsidianlog query` against an archive with two batches in
one window returned the second batch's content twice and never returned the
first batch's records at all — silent, permanent data loss on the very
guarantee (long-term tamper-evident archival) the project exists to provide.
The hash chain itself was unaffected (`Chunk` objects are chained by content,
not by path), which is why `verify` did not catch this: the chain machinery
never observed the collision, only `StorageBackend`'s file/object layer did.

Rejected fix: merge new records into the existing chunk for the window
instead of writing a second one. This contradicts the append-only,
immutable-once-chained design in ADR-0003 — a chunk's hash is computed once
over its final ciphertext and linked into the chain; rewriting a
already-chained chunk's contents after the fact is exactly what tamper
evidence exists to detect and forbid, regardless of who's doing the
rewriting.

## Decision

Extend the storage key from `(service, window)` to `(service, window,
sequence)`, matching what the manifest already tracked per `ChunkRef`.
`StorageBackend::get_chunk`/`get_index` both take an explicit `sequence: u64`
parameter, and both backends fold it into the physical path:

- `LocalBackend`: `chunks/<service>/<window>-<sequence>.bin`,
  `index/<service>/<window>-<sequence>.idx`
- `SiaBackend`: `<bucket>/chunks/<service>/<window>-<sequence>.bin` (object
  metadata path, per ADR-0006)

Every write within a window now lands at a distinct path by construction —
the same collision-by-construction guarantee ADR-0009 established for
nonces, applied here to storage paths. `ArchiveEngine` callers
(`read_records`, `service_chunks`, `query`, `verify_service`) thread
`chunk_ref.sequence` through from the manifest, which already had it.

Along the way, `obsidianlog-core::types::ChunkId` — a dead struct whose
`chunk_path()`/`index_path()` asserted the old, incorrect `(service,
window)`-only format, unused by either real backend — was deleted rather than
updated, to avoid leaving a second, misleading source of truth for the path
format.

## Consequences

- Fixes silent, permanent data loss for any service that receives more than
  one batch per window — the common case, not a rare one.
- `SiaBackend` object count grows by one object per batch-in-a-window rather
  than one per window. ADR-0008 already identified `chunking.window_secs` as
  the user-facing lever for controlling Sia object count/cost; that guidance
  now also covers batch frequency within a window, and is the accepted
  tradeoff for correctness over object-count minimization.
- No manifest format change — `ChunkRef.sequence` already existed and is now
  load-bearing for reads, not just for the hash chain and nonce derivation.
- Regression coverage: `LocalBackend`'s
  `same_window_different_sequence_does_not_collide` proves two batches in one
  window no longer collide on disk.
