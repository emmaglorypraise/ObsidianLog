# 0003 — Per-service chains and serialized writes

- Status: Accepted
- Date: 2026-06-29

## Context

Tamper-evidence comes from hash-chaining chunks: each chunk records
`prev_hash = SHA-256(previous chunk)` over its canonical bytes (ADR-0005), so any
deletion, reorder, or modification breaks the chain at a detectable position.

Two questions follow from that: how many chains do we keep, and how are writes to
a chain coordinated? ObsidianLog ingests many independent log streams (one per
service) and wants high write throughput, but a hash chain is inherently
sequential — each link depends on the one before it.

## Decision

### Chains are per service

We maintain **one independent hash chain per service**, not a single global
chain. The `Manifest` holds one `ManifestServiceChain` per service, each with its
own `head_hash` and `next_sequence` (ADR-0005). A service's first chunk uses the
all-zero `GENESIS` as its `prev_hash`.

A single global chain would impose a global write order: every chunk, regardless
of which service produced it, would have to append after the current global head,
serializing *all* ingest behind one lock and coupling unrelated services'
throughput. Per-service chains let **services ingest in parallel** — each chain is
contended only by its own service's writes.

### Writes within a service are serialized

Within one service, writes **must** be serialized. Computing a chunk's
`prev_hash` requires the hash of the immediately preceding chunk, and that chunk's
hash isn't known until it has been sealed. Two concurrent appends to the same
service would both read the same head and produce two chunks with the same
`prev_hash` and sequence number — a fork, not a chain. So each service has a
single writer (a `chain::ChainBuilder` drives one service's appends), assigning
monotonic sequence numbers and linking `prev_hash` in order.

This per-service serialization is also what makes the deterministic nonce counter
safe (ADR-0002): the per-service `next_sequence` has exactly one writer.

### The Manifest holds each chain's head and counter

The `Manifest` is the durable record of, per service, the current chain
`head_hash` and the `next_sequence` to assign. On startup the writer resumes from
the manifest (`ChainBuilder::resume`) so new chunks link onto the existing head
and continue the sequence — never restarting at genesis and never reusing a
sequence number (and therefore never reusing a nonce).

## Consequences

- Parallel ingest across services with no global write lock; contention is bounded
  to a single service's chain.
- Each service needs a single serialized writer; a multi-writer ingest path must
  shard or lock per service.
- `verify_chain` walks one service's chain independently and can be scoped to a
  single service (`obsidianlog verify --service <name>`); a break is localized to
  and reported within the affected service.
- The `Manifest` (head + counter) is safety-critical and must be persisted
  durably and recovered before the next append — see ADR-0002 and the backend
  durability contract (ADR-0004/0005).
- There is no global ordering across services; if cross-service ordering is ever
  needed, it must come from record timestamps in the index, not the chain.
