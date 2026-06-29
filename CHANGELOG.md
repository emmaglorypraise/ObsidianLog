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

[Unreleased]: https://github.com/emmaglorypraise/ObsidianLog/commits/main
