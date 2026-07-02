# 0006 — Sia integration (feature-gated, pinned SDK)

- Status: Accepted
- Date: 2026-07-02

## Context

ObsidianLog's whole point is archiving to Sia, but the Rust SDK for that —
`sia_storage` (part of [`sia-sdk-rs`](https://github.com/SiaFoundation/sia-sdk-rs))
— is **pre-1.0** and ships breaking changes on minor bumps. We want the real
integration without letting an unstable, heavy dependency destabilize the
default build, tests, or CI (the mock-first invariant, ADR-0004).

We also found that the SDK is not shaped like the S3-style, path-keyed object
store our `StorageBackend` trait assumes.

## Decision

### Feature-gated behind `sia`

The entire Sia backend lives in `obsidianlog-store::backend::sia`, compiled only
under a non-default cargo feature `sia`:

```toml
sia_storage = { version = "=0.10.0", optional = true }
# ...
[features]
default = []
sia = ["dep:sia_storage", "tokio/io-util"]
```

`cargo build`, `cargo clippy`, and `cargo test` with default features pull **no**
Sia dependency and compile Sia-free. Only `--features sia` brings in
`sia_storage` and its tree (`sia_core`, `sia_mux`, `reqwest`, crypto crates).

### Pinned to an exact version

`sia_storage` is pinned to **`=0.10.0`**, not a caret range. Because the SDK is
pre-1.0, a `^0.10` range would silently accept `0.10.x` — and, worse, tooling
treats pre-1.0 minors as breaking. An exact pin means upgrades are a deliberate,
reviewed act, never an accidental `cargo update`. (A git-revision pin is the
alternative if we ever need an unpublished fix; the version pin is preferred
while the crate is published.)

### TLS via rustls

The SDK's `reqwest` is configured with `rustls-tls` (verified: the dependency
tree contains `rustls`/`hyper-rustls`/`tokio-rustls`, no `native-tls`/OpenSSL),
satisfying our rustls-everywhere rule with no extra configuration.

### Mapping onto a content-addressed store

`sia_storage` is **content-addressed**: `upload` returns an `Object` keyed by a
derived hash, fetched with `Sdk::object(&Hash256)` — there is no get-by-path, and
"list" is an `object_events` sync stream. Each object does carry an arbitrary
`metadata: Vec<u8>`. So `SiaBackend` stores each object's bucket-relative path
(`<bucket>/chunks/<service>/<window>.bin`, …) in that metadata and resolves reads
by scanning `object_events` for a matching path. This keeps `LocalBackend` and
`SiaBackend` interchangeable behind `StorageBackend`. Reads are `O(objects)`; a
future optimization records each chunk's object id in the manifest for direct
`Sdk::object` fetches.

Connection uses the reconnect path (`Builder::connected(&AppKey)`) with a saved,
pre-approved `AppKey`; the interactive first-time approval flow is out of scope
for a backend.

## Consequences

- Default builds/tests/CI stay fast and Sia-free; the pre-1.0 risk is quarantined
  behind one feature and one module.
- The Sia backend is **compile-verified in CI but not run** there: its
  integration test is gated behind `--features sia` **and**
  `OBSIDIANLOG_INDEXD_URL`/`OBSIDIANLOG_APP_KEY`, skipping cleanly without a live
  indexer.
- **Upgrading deliberately:** bump the `=0.10.0` pin, run
  `cargo build --features sia` and `cargo clippy --all-targets --features sia`,
  re-read the SDK's `CHANGELOG` for breaking changes to the upload/download/object
  APIs used in `sia.rs`, and (with an indexer) run the env-gated integration test
  before merging. Record notable API shifts by superseding this ADR.
- Objects are encrypted twice (our AES-256-GCM plus the SDK's) — harmless
  defense-in-depth.
