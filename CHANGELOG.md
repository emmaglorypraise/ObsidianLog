# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Cargo workspace scaffold with four crates:
  - `obsidianlog-core` — shared types, the canonical error, and the
    `StorageBackend` trait, kept free of I/O so the trait stays apart from any
    implementation.
  - `obsidianlog-store` — storage pipeline (compression, encryption, hash
    chaining, chunking, metadata index, manifest) plus the backend impls: a
    default `LocalBackend` and a `SiaBackend` gated behind the `sia` feature.
  - `obsidianlog-ingest` — Vector-compatible HTTP ingest service (axum/tokio).
  - `obsidianlog-cli` — the `obsidianlog` binary (`init`, `ingest`, `query`,
    `verify`), plus config loading and the OS-keychain key store.
  - All feature logic is stubbed with `todo!()` / `TODO(impl)` markers.
- Mock-first design: default builds and tests run entirely against
  `LocalBackend` with no Sia node; the pre-1.0 Sia SDK is confined to the
  `sia`-feature-gated backend.
- Security invariants encoded in the scaffold: deterministic per-service-counter
  AES-GCM nonces (no random reuse risk) and per-service hash chains for parallel
  ingest (see ADR-0002, ADR-0003).
- Integration-test scaffolds for the store, ingest, and CLI crates.
- Tooling: MIT license, README with CI/audit badges, toolchain pinning
  (`rust-toolchain.toml`), `rustfmt.toml`, and `.gitignore`.
- CI (fmt + clippy + cross-platform test matrix + `cargo audit`) and a
  cross-platform binary release workflow.
- Docs: Architecture Decision Records, CONTRIBUTING, SECURITY, and a Docker
  Compose quickstart.
- Contribution standards: Conventional Commits and branch-naming conventions in
  CONTRIBUTING, plus a `.gitmessage` template (CI enforcement deferred to the
  Month 3 pre-launch milestone).
- Community health files: Code of Conduct (Contributor Covenant 2.1), issue
  forms (bug/feature) with a contact-links config, a pull-request template, a
  Dependabot config (cargo + GitHub Actions), and an `.editorconfig`.
- Sia Foundation grant workflow: an architecture overview (diagram) in the
  README, a monthly progress report (`docs/grant/PROGRESS.md`), and a
  pull-request template that asks for testing instructions and remaining-work
  notes, per the Foundation's Grants Development Guide.
- `obsidianlog-core` data model: `LogRecord`/`LogBatch`, `ChunkHeader`/`Chunk`
  (with a canonical, golden-tested hashing layout), `ChunkRef`,
  `ServiceWindowIndex`, `ManifestServiceChain`/`Manifest`, and a restructured
  `Error`. The `StorageBackend` trait is now async (`async-trait`) with
  chunk/index/manifest methods and a documented durability contract. See
  ADR-0005.
- `obsidianlog-store::compress`: zstd `compress`/`decompress` (default level 3),
  the first implemented pipeline stage.
- `obsidianlog-store::encrypt`: AES-256-GCM `encrypt_chunk`/`decrypt_chunk` and a
  zeroize-on-drop `EncryptionKey` newtype. Nonces are caller-supplied and derived
  from the per-service sequence counter (never random); see ADR-0002. Renamed the
  `encryption` stub module to `encrypt`.
- `obsidianlog-store::chain`: SHA-256 `compute_chunk_hash`, a per-service
  `ChainBuilder` (assigns sequence + `prev_hash`), and `verify_chain` reporting
  the position and kind (modified/reordered/missing) of the first break. See
  ADR-0003.
- `obsidianlog-store::chunking`: groups a `LogBatch` into per-`(service, window)`
  buckets with `YYYY-MM-DD-HH` labels (configurable window, default 1 hour).
- Completed the store→core migration: removed the duplicate `index`/`manifest`
  stub modules (core owns those types) and renamed `hashchain`→`chain`,
  `chunk`→`chunking`.
- `obsidianlog-store::parse`: tolerant Vector-event parsing into `LogRecord`s
  (missing fields → `None`, unparseable timestamp → ingest time with a flag,
  malformed input never panics).
- `obsidianlog-store::index`: builds the lightweight `ServiceWindowIndex`
  (min/max time, level/host sets, keyword tokens) per chunk, plus a conservative
  `might_match` prefilter over `IndexQuery`.
- `obsidianlog-store::backend::LocalBackend`: filesystem implementation of the
  `StorageBackend` trait under `<root>/<bucket>` (chunks/index/manifest). Writes
  are atomic and durable (temp file → fsync → rename → dir fsync); reads return
  `Error::NotFound` when missing; `list_chunks` filters by service and optional
  time range; `update_manifest` serializes concurrent manifest updates. Chunks
  use a compact `u32(header) || header-json || ciphertext` frame.
- `obsidianlog-store::ArchiveEngine`: the end-to-end pipeline over any
  `StorageBackend`. `ingest_batch` chunks → compresses → encrypts → hash-chains →
  indexes → persists, per-service serialized (per-service async lock plus a
  global manifest lock), acknowledging only after durable writes. Adds
  `encrypt::derive_nonce(service, sequence)`. The two `tests/pipeline.rs`
  integration tests (round-trip, tamper-detection) are now live.
- `obsidianlog-store::backend::SiaBackend`: a `StorageBackend` over the Sia
  Foundation's `sia_storage` SDK (indexd app API), behind a non-default `sia`
  cargo feature (default builds/tests stay Sia-free). The SDK is content-
  addressed, so our `(service, window)` paths are stored in object metadata and
  resolved by scanning `object_events`. `sia_storage` is pinned to `=0.10.0` and
  uses rustls; an env-gated integration test (`OBSIDIANLOG_INDEXD_URL`) reuses the
  milestone end-to-end assertions. See ADR-0006.

[Unreleased]: https://github.com/emmaglorypraise/ObsidianLog/commits/main
