# 0004 — Workspace layout and storage abstraction

- Status: Accepted
- Date: 2026-06-26

## Context

The proposal's deliverables name a storage library, an HTTP ingest service, and
a CLI. Three cross-cutting concerns shape the structure:

1. **Publishability.** The open-source core (storage library, ingest, CLI) is to
   remain MIT-licensed and reusable, which means real, independently publishable
   crates.
2. **Sia SDK maturity.** The Sia integration depends on the pre-1.0 `sia_storage`
   SDK. The project commits to "modular storage abstractions to minimize coupling
   to any single integration layer," and default builds/tests must not require a
   Sia node or that SDK.
3. **Mock-first development.** Everything must be buildable and testable against a
   local backend, so the riskiest dependency never blocks day-to-day work.

## Decision

Use a **Cargo workspace** with four members:

- `obsidianlog-core` — shared vocabulary: domain types, the canonical
  `Error`/`Result`, and the `StorageBackend` trait. No I/O. Keeping the trait
  here, apart from any implementation, is what prevents the Sia SDK from leaking
  into the pure pipeline.
- `obsidianlog-store` — the storage pipeline (zstd compression, AES-256-GCM
  encryption, SHA-256 hash chaining, append-only chunking, metadata index,
  manifest) **and** the backend implementations: a default `LocalBackend` and a
  `SiaBackend` compiled only under the `sia` feature.
- `obsidianlog-ingest` — the Vector-compatible HTTP ingest service, depending on
  `obsidianlog-store`.
- `obsidianlog-cli` — the CLI; builds the `obsidianlog` binary, depending on the
  others.

All durable-storage I/O goes through the `core` `StorageBackend` trait.
`LocalBackend` (filesystem) is the default so the default build and `cargo test`
need no Sia node; the pre-1.0 `sia_storage` SDK is confined to `SiaBackend`
behind the `sia` feature. Shared metadata and common dependencies are
centralized in `[workspace.package]` and `[workspace.dependencies]`.

The storage layout is fixed (mirrored by `LocalBackend` under its data dir and by
`SiaBackend` under the bucket):

```
<root>/
  index/<service>/<YYYY-MM-DD-HH>-<sequence>.idx   # lightweight metadata, fetched first
  chunks/<service>/<YYYY-MM-DD-HH>-<sequence>.bin  # encrypted + compressed log data
  manifest/...                                      # per-service chain heads + chunk refs
```

(The `-<sequence>` suffix was added in ADR-0010: `(service, window)` alone is not
a unique storage location when a window receives more than one batch.)

## Consequences

- The crypto/chunking core is testable against `LocalBackend` and is insulated
  from Sia SDK churn — if the SDK changes, only `backend/sia.rs` moves.
- Default builds and CI are fast and dependency-light; the `sia` feature is the
  single opt-in for the network integration.
- Each crate is independently versioned and publishable to crates.io. The
  published CLI crate is `obsidianlog-cli`; it installs the `obsidianlog` binary.
- Slightly more boilerplate (four manifests, a re-export layer in `store`) than a
  single crate, accepted for the isolation, testability, and publishability
  benefits.
- Crate-specific heavy dependencies (axum/tokio in ingest; the RustCrypto stack
  in store) stay out of crates that don't need them.
