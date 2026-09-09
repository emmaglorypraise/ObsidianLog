# 0014 — Advisory lock enforces the single-writer assumption per data directory

- Status: Accepted
- Date: 2026-09-09

## Context

ADR-0003 establishes that each service needs exactly one writer: two
concurrent appends to the same service would read the same chain head and
produce two chunks with the same `prev_hash` and sequence number (a fork),
and — because the nonce is derived from that same sequence number (ADR-0002)
— also reuse a nonce. `ArchiveEngine` enforces this with in-memory locks
(`service_locks`, `manifest_lock`), but those locks live inside one process.

Nothing stopped a second, independent process from pointing at the same
`LocalBackend` data directory. Two `obsidianlog serve` instances started
against the same `--config` (accidentally, via a bad supervisor restart
policy, or a misconfigured second deployment) would each hold their own
in-memory locks, see no conflict with each other, and both write to the same
`manifest.json` — reintroducing exactly the fork/nonce-reuse failure mode
ADR-0003 and ADR-0002 exist to prevent, just across processes instead of
within one. Flagged by the Month 2 reviewer: nothing detected or prevented
this.

## Decision

`LocalBackend::acquire_write_lock` takes an exclusive, non-blocking OS
advisory lock (`flock` on Unix, `LockFileEx` on Windows, via the `fs4` crate
— pure Rust, no OpenSSL, matching the project's rustls-only policy) on a
`.lock` file inside the data directory. `obsidianlog serve` acquires this
once at startup, before constructing the `ArchiveEngine`, and holds the guard
for the server's entire lifetime; the OS releases the lock automatically
when the guard drops (process exit, including a crash).

`AnyBackend::acquire_write_lock` dispatches this for the `Local` variant and
is a no-op (`Ok(None)`) for `Sia` — a hosted indexer already serializes
writes on its own (out of scope for this decision, which is specifically
about the shared local filesystem).

A second `obsidianlog serve` (or any other process calling
`acquire_write_lock` on the same directory) gets a clear error immediately at
startup — `another obsidianlog process is already writing to <dir>` —
instead of starting successfully and corrupting the manifest later. `query`
and `verify` do **not** take this lock: they're read-only, don't touch
`manifest.json`'s writer-side invariants, and should keep working
concurrently with a running `serve` (e.g. querying live-ingesting logs).

The README now documents the single-writer assumption explicitly, alongside
ADR-0003's per-service-writer rule it extends to the process level.

## Consequences

- A second `obsidianlog serve` against the same data directory fails fast
  and loudly at startup, closing the reviewer-flagged gap.
- No protection against a *different* kind of second writer: a process that
  writes into the data directory without going through `LocalBackend`
  entirely (e.g. a stray script directly manipulating `manifest.json`).
  Advisory locks only bind cooperating callers; this was never in scope —
  the fix is specifically about two `obsidianlog` instances.
- `query`/`verify` remain lock-free by design, so they stay usable while
  `serve` is running; they were never part of the write-path race this
  closes.
- The Sia backend is untouched by this decision. If a future milestone needs
  the same guarantee for concurrent writers against a shared hosted
  indexer, that's a separate decision informed by `indexd`'s own concurrency
  model, not an extension of this file-based lock.
