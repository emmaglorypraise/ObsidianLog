# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- `Config::default_path()` failed on every default Windows shell (`error:
  could not determine the config directory: neither XDG_CONFIG_HOME nor
  HOME is set`), since PowerShell and cmd.exe set `USERPROFILE`, not
  `HOME`. Now falls back to `USERPROFILE` when `HOME` is unset. Found via
  two independent external Windows testers hitting it on a fresh install.
- `default_key_store`'s fallback to the local secrets file only triggered
  on `keyring::Error::NoStorageAccess`, missing a distinct error variant
  (`NoDefaultStore`) that a headless Linux environment with no D-Bus Secret
  Service running (bare CI runners, many minimal containers) returns
  instead — no store could be reached at all, so `obsidianlog init`
  hard-failed rather than falling back. Broke the `demo.yml` CI pipeline,
  and would break the same way for real headless-Linux use.

## [0.2.0] - 2026-09-18

### Breaking

- Local credential storage changed (ADR-0015): the archive's encryption key
  and the Sia app key are now stored together as **one** bundled OS-keychain
  item, replacing the two independent items used through 0.1.x. A config
  file written by an older release fails to load with a clear message
  rather than being silently reinterpreted — there is no automatic
  migration. **To keep reading archives created under 0.1.x, stay on
  `obsidianlog` v0.1.1.** To move to 0.2, run `obsidianlog init --force`:
  this generates a new encryption key, so previously archived data will no
  longer be decryptable under it.

### Fixed

- A fresh `obsidianlog init` (local or Sia) produced multiple separate
  macOS Keychain authorization prompts, one per keychain call. The new
  bundled-credential storage, combined with a direct "create only" macOS
  Keychain write instead of the generic check-then-write pattern, brings
  this down to exactly one prompt for a clean fresh install (local or Sia
  alike) and for a plain repair (config file missing, credential still
  present). A repair that also introduces new credential material — e.g.
  choosing Sia while an existing bundle was local-only — costs more than
  one operation by design, prioritizing correctness (the existing
  encryption key is always read and preserved exactly) over shaving that
  rarer case's call count.
- `default_key_store` (credential-store resolution) treated any error from
  the OS keychain, including the user cancelling or denying an
  authorization prompt, as "keychain unavailable, fall back to a plain
  file instead." Only genuine unavailability now triggers that fallback;
  a cancellation or denial surfaces as a real error.
- `obsidianlog init --force` read the credential bundle twice: once in a
  preflight "is setup already complete?" check whose result was then
  discarded, and again inside the rotation itself to preserve any existing
  Sia key. The preflight read is now skipped when `--force` is already
  explicit, cutting a forced rotation from three keychain operations down
  to two (or to a single write when a new Sia key is chosen in the same
  run).
- Onboarding to Sia only validated the recovery phrase *after* the browser
  approval step completed, so a mistyped phrase (including `SEED` in the
  wrong case) burned a full approval round-trip — and one of a
  possibly-limited number of app-connection slots on the indexer — before
  failing with a generic parse error. The phrase is now validated locally
  first, and `seed` is matched case-insensitively.
- A re-run of `obsidianlog init` against an existing but incomplete setup
  (or one where `Reuse it?` is declined) re-probed the keychain with a
  `create()` attempt and a second `read()`, even though the bundle's
  contents were already read once for the integrity/reuse check moments
  earlier. That known bundle is now reused directly instead of being
  discarded and re-fetched, cutting this case from up to four keychain
  operations down to at most two.

### Changed

- A repair that finds `config.toml` missing but the credential bundle
  still present (e.g. after accidentally deleting just the config file)
  re-collects every wizard answer — there's no config left to read the old
  ones from — which read as "did this just reset my key?" `obsidianlog
  init` now prints a status line making the outcome explicit: "Existing
  credentials preserved; rebuilding configuration." for a plain reuse, or
  "Existing encryption key preserved; Sia app key saved." when the repair
  also adds a Sia app key.
- The official release binaries now build `obsidianlog` with the `sia`
  Cargo feature enabled, instead of shipping local-only and pointing Sia
  users at a source build. `obsidianlog init` still defaults to the local
  backend regardless — this only changes what's compiled in, not what
  happens without choosing Sia explicitly. `obsidianlog-ingest` (the
  standalone, minimal ingest-only binary) stays local-only, unchanged.

## [0.1.1] - 2026-08-18

### Fixed

- `obsidianlog init` made up to 6 redundant OS keychain calls in a single
  run (two exact-duplicate existence checks, plus a delete that ran
  unconditionally right before an overwrite that already made it a no-op),
  each able to trigger its own macOS authorization prompt. A fresh install
  now makes 3 calls (4 if the Sia backend is chosen), and `--force` key
  rotation drops from 6 to 3. No behavior change.
- The README's release-binary install instructions used literal
  `vX.Y.Z-<target>` placeholder syntax that broke when copy-pasted into a
  shell (`<`/`>` are redirection operators, not just visual placeholders).
  Replaced with a real, tested example plus a note to swap in the actual
  downloaded filename.

### Changed

- Cleaned up README prose for readability throughout: shorter sentences,
  em dashes replaced with clearer punctuation. No content or command
  changes.
- Documented that macOS's `init` keychain prompt should be answered with
  "Always Allow" to avoid repeat prompts on later `serve`/`query`/`verify`
  runs, and that canceling it aborts the whole `init` run rather than
  resuming.

## [0.1.0] - 2026-08-17

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
  - The `obsidianlog` CLI subcommands and key/config plumbing are stubbed —
    they return a clear "not yet implemented" error, pending Month 2.
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
  uses rustls; an env-gated integration test
  (`OBSIDIANLOG_INDEXD_URL`/`OBSIDIANLOG_APP_KEY`) reuses the milestone
  end-to-end assertions and has been **run successfully against real Sia**
  (`sia.storage`). See ADR-0006.
- `obsidianlog-store` Sia app onboarding: a real, stable App ID in `APP_META`,
  and an `onboard` example (`--features sia`) that runs the indexer approval flow
  to export an `AppKey` (recovery phrase read from stdin; never stored). See
  ADR-0006.
- `obsidianlog-ingest`: a Vector-compatible HTTP ingest server (axum/tokio).
  `POST /ingest` accepts a JSON array of events (or NDJSON), parses tolerantly,
  and archives via `ArchiveEngine` — **write-then-ack** (`200` only after a
  durable write; `5xx` on failure so Vector retries; `400` on malformed input).
  `GET /health` for readiness. Config (bind/bucket/storage-root/window) loads from
  a TOML file or defaults; ships a thin `obsidianlog-ingest` binary and an
  `examples/vector.toml`. Integration tests boot the server on an ephemeral port,
  POST over reqwest (rustls), and verify the archived batch. The workspace now has
  **zero ignored tests**.

### Changed

- Bumped dependencies to current releases: `aes-gcm` 0.11, `toml` 1.x, `zeroize`
  1.9, `keyring` 4.1.4.
- Recorded ADR-0007 (indexer topology: hosted-default, bring-your-own-indexer).

[Unreleased]: https://github.com/emmaglorypraise/ObsidianLog/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/emmaglorypraise/ObsidianLog/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/emmaglorypraise/ObsidianLog/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/emmaglorypraise/ObsidianLog/releases/tag/v0.1.0
