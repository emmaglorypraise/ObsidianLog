# Grant progress — ObsidianLog

Development tracker for the Sia Foundation grant: each milestone's deliverables,
broken into tasks, with the pull request(s) — or commit(s) — that implement them,
following the Foundation's Grants Development Guide.

> **Canonical source.** The approved goals, deliverables, and success criteria
> live in the grant proposal on the **Sia Foundation forum** (the source of
> truth). This file is the in-repo development tracker; the milestone summaries
> below mirror the proposal for convenience.

> **Reporting note.** Early scaffolding work predates the project's switch to a
> PR-based flow, so those rows link **commits** rather than PRs (permitted for
> pre-existing grants by the guide's caveat). Every task from here on lands as a
> pull request and links the PR number. At month end, the completed rows for that
> month's milestone are the report submitted to the forum.

## Goal

The MVP answers one core question: **can real logs flow from a production-grade
log agent into Sia — encrypted, compressed, and tamper-proof — and be retrieved
accurately through a query interface?** Everything beyond that is deferred to
Phase 2.

## Timeline

| Milestone | Theme | Due |
| --- | --- | --- |
| Month 1 | Core Storage & Ingestion | 2026-07-25 |
| Month 2 | Query Tooling & Developer Experience | 2026-08-25 |
| Month 3 | Launch & Ecosystem Integration | 2026-09-25 |

## Foundational setup (pre-milestone)

Project scaffolding and process setup, completed before milestone task work.
Commit-linked (pre-PR), per the pre-existing-grant caveat.

| Task | Commit(s) | Notes |
| --- | --- | --- |
| Workspace scaffold: 4 crates (core/store/ingest/cli), mock-first backends, CI, ADRs | `3ceb7ca` | Pipeline stubbed with `todo!()`; not yet functional. |
| Commit-convention docs (add `core` scope) | `132c0c3` | |
| Sia grant workflow + progress tracking | `969a293` | |
| Security practices documentation | `0ae9c05` | |

## Month 1 — Core Storage & Ingestion (due 2026-07-25)

**Goal:** Build the production core storage library and production-ready HTTP
ingest server.

| Deliverable / task | Pull Request(s) / Commit(s) | Status / Notes |
| --- | --- | --- |
| `obsidianlog-core`: data model + async `StorageBackend` trait | [#3](https://github.com/emmaglorypraise/ObsidianLog/pull/3) | _Done._ Foundation for the storage library. Canonical hashing layout + per-service chains; ADR-0005. |
| `obsidianlog-store`: zstd compression | [#5](https://github.com/emmaglorypraise/ObsidianLog/pull/5) | _Done._ `compress`/`decompress`, default level 3; round-trip + ratio tests. |
| `obsidianlog-store`: AES-256-GCM encryption + deterministic nonces | [#6](https://github.com/emmaglorypraise/ObsidianLog/pull/6) | _Done._ `encrypt_chunk`/`decrypt_chunk`, zeroize key, caller-supplied counter nonce; ADR-0002. |
| `obsidianlog-store`: SHA-256 per-service hash chaining (`chain`) | [#7](https://github.com/emmaglorypraise/ObsidianLog/pull/7) | _Done._ `compute_chunk_hash`, `ChainBuilder`, `verify_chain`; ADR-0003. Manifest persistence lands with `LocalBackend`. |
| `obsidianlog-store`: time-window `chunking` | [#7](https://github.com/emmaglorypraise/ObsidianLog/pull/7) | _Done._ `chunk_batch` into per-`(service, window)` buckets. |
| `obsidianlog-store`: log parsing + metadata index (`parse`, `index`) | [#8](https://github.com/emmaglorypraise/ObsidianLog/pull/8) | _Done._ Tolerant Vector-event parsing; `ServiceWindowIndex` + `might_match` prefilter. |
| `obsidianlog-store`: `LocalBackend` (Sia-free) | [#9](https://github.com/emmaglorypraise/ObsidianLog/pull/9) | _Done._ Atomic+durable filesystem store; not-found reads; ranged `list`; serialized manifest updates. |
| `obsidianlog-store`: end-to-end archive pipeline (`ArchiveEngine`) | [#10](https://github.com/emmaglorypraise/ObsidianLog/pull/10) | _Done._ `ingest_batch` chunk→compress→encrypt→chain→index→persist, per-service serialized; milestone round-trip test. |
| `obsidianlog-store`: feature-gated `SiaBackend` (`sia_storage`) | [#11](https://github.com/emmaglorypraise/ObsidianLog/pull/11), [#21](https://github.com/emmaglorypraise/ObsidianLog/pull/21) | _Done._ Real Sia backend behind the `sia` feature (pinned `=0.10.0`, rustls). **Verified end-to-end against real Sia** (`sia.storage`): ingest → upload → read back → chain-verify. App onboarding + real App ID via the `onboard` example (#21). Finding: 16 tiny records provisioned ~503 MB over ~17 min (Sia per-object overhead — batching optimization noted for later). ADR-0006. Brings Milestone-3 Sia work forward. |
| `obsidianlog-ingest`: Vector-compatible HTTP ingest server (`/ingest`, `/health`) | [#13](https://github.com/emmaglorypraise/ObsidianLog/pull/13) | _Done._ axum write-then-ack server + thin binary + `examples/vector.toml`; ephemeral-port reqwest integration tests. |
| Integration test suite with CI | [#13](https://github.com/emmaglorypraise/ObsidianLog/pull/13) | _Done._ CI green across 3 OSes + audit; **zero ignored tests** — store end-to-end + ingest HTTP tests all live. |
| ADR documenting finalized storage decisions | _done_ | `3ceb7ca` — ADR-0002 (nonces), ADR-0003 (chains), ADR-0004 (layout). |
| Month 1 progress report submitted to the Sia Foundation forum | _done_ | Submitted (was due 2026-07-25). |

## Month 2 — Query Tooling & Developer Experience (due 2026-08-25)

**Goal:** Build retrieval tooling, query interface, and onboarding experience.

| Deliverable / task | Pull Request(s) / Commit(s) | Status / Notes |
| --- | --- | --- |
| `obsidianlog` CLI command skeleton + config loading | [#32](https://github.com/emmaglorypraise/ObsidianLog/pull/32) | _Done._ `init`/`serve`/`query`/`verify` clap tree; `--config` precedence (flag > `$XDG_CONFIG_HOME` > `~/.config`), TOML loading; `serve` wired to `obsidianlog-ingest::serve`. |
| `obsidianlog` CLI query interface | [#33](https://github.com/emmaglorypraise/ObsidianLog/pull/33) | _Done._ Index-first `ArchiveEngine::query`: candidate windows from the manifest by time+service, prefiltered via `ServiceWindowIndex`, exact per-record filtering; `--from`/`--to`/`--service`/`--level`/`--host`/`--keyword`, `human`/`json`/`raw` output. |
| Hash-chain verification tooling (`obsidianlog verify`) | [#34](https://github.com/emmaglorypraise/ObsidianLog/pull/34) | _Done._ `ArchiveEngine::verify_service`/`verify_all` walk each service's chain from genesis and check it against the manifest's recorded head; never decrypts. Optional `--service` scope; non-zero exit on any failure for CI/cron. |
| `obsidianlog init` setup wizard | [#38](https://github.com/emmaglorypraise/ObsidianLog/pull/38) | _Done._ Interactive (or `--non-interactive`) setup: generates the encryption key, writes `config.toml`; idempotent, with `--force` to rotate. `serve`/`query`/`verify` now work end-to-end against a real init'd install. |
| Reviewer-flagged fix: cross-service nonce discriminator (AES-GCM) | [#39](https://github.com/emmaglorypraise/ObsidianLog/pull/39) | _Done._ The Month 1 reviewer linked a PoC showing the `SHA-256(service)[..4]` discriminator could collide between two service names (attacker-influenceable, ~2^32 search), reusing a `(key, nonce)` pair. Fixed: each service now gets a manifest-assigned unique id, making cross-service uniqueness unconditional. ADR-0009 (amends ADR-0002); breaking `MANIFEST_VERSION` bump (1→2, no migration needed — no production deployments yet); regression test proves the old exploit scenario no longer works. Fixes Month 1 (`obsidianlog-store`) code; landed in the Month 2 window. |
| CLI backend selection: `serve`/`query`/`verify` route to Local or Sia | [#44](https://github.com/emmaglorypraise/ObsidianLog/pull/44) | _Done._ Pulls forward the "make the ingest server's backend selectable" half of Month 3's indexer-topology item (ADR-0007) — a prerequisite the Month 1 reviewer's note surfaced ahead of schedule. `obsidianlog-store` gains `AnyBackend`, a runtime-selected `StorageBackend` (Local, or Sia behind the `sia` feature); `obsidianlog-ingest`'s server is now generic over the backend instead of hardcoded to `LocalBackend`; the CLI's `resolve_backend()` picks Local when `config.indexd` is unset, otherwise connects to Sia using the app key PR #43 wired up (or errors clearly if the `sia` feature isn't compiled in). Verified end-to-end against the local backend (init → serve → query → verify, including a real ingest through the HTTP endpoint); the Sia path is structurally verified (builds/lints clean under `--features sia`) but not yet exercised against a live indexer connection — operating a hosted indexer is still open, tracked under Month 3's indexer-topology row. |
| Fix: chunk storage path collision caused silent data loss | [#45](https://github.com/emmaglorypraise/ObsidianLog/pull/45) | _Done._ Found via manual testing: `obsidianlog query` returned one batch's content twice. Root cause — `get_chunk`/`get_index` and both backends keyed storage paths by `(service, window)` alone, so a window that received more than one ingest batch overwrote (`LocalBackend`) or permanently orphaned (`SiaBackend`, while still billing for it) the earlier batch. Fixed by folding the manifest's existing per-chunk `sequence` into the storage key. ADR-0010; regression test proves two batches in one window no longer collide. Fixes Month 1 (`obsidianlog-store`) code; landed in the Month 2 window. |
| Fix: standalone `obsidianlog-ingest` binary could run with a hardcoded placeholder key | [#46](https://github.com/emmaglorypraise/ObsidianLog/pull/46) | _Done._ Found via security review. The standalone binary — a documented, supported way to run the server — never loaded a real encryption key and only warned (rather than refusing to start) when running with the all-zero placeholder, silently encrypting archived logs under a publicly-known key. `obsidianlog serve` (the CLI) was never affected — it already always loads a real key from the OS keychain. `serve()` now hard-errors on the placeholder; the standalone binary supplies a real key via `OBSIDIANLOG_ENCRYPTION_KEY_FILE` (mounted secret, preferred) or `OBSIDIANLOG_ENCRYPTION_KEY`. ADR-0011. Fixes Month 1 (`obsidianlog-ingest`) code; landed in the Month 2 window. |
| Cross-platform binaries (Linux, macOS, Windows) | [#51](https://github.com/emmaglorypraise/ObsidianLog/pull/51), [#52](https://github.com/emmaglorypraise/ObsidianLog/pull/52), [#53](https://github.com/emmaglorypraise/ObsidianLog/pull/53) | _Done._ `release.yml` builds `obsidianlog` + `obsidianlog-ingest` for 5 targets (Linux musl x86_64/aarch64, macOS x86_64/aarch64, Windows msvc) on a pushed `v*` tag, gated on a `cargo tree` check that no OpenSSL/native-tls crept into the dependency tree. Validated against two real pre-release tags (`v0.1.0-rc.1` found a false-positive in the musl static-link check, `v0.1.0-rc.2` confirmed the fix — all 5 targets green, artifacts attached; both test tags deleted after). #53 documents installing a release binary in the README. |
| Docker Compose quickstart | [#48](https://github.com/emmaglorypraise/ObsidianLog/pull/48) | _Done._ `docker/docker-compose.yml` runs the ingest server against the local backend by default (no Sia node needed); real Sia storage via `indexd` + Postgres is available behind an opt-in `sia` Compose profile. `docker/Dockerfile` builds the `obsidianlog` binary with the `sia` feature and a healthcheck; README documents the full init → ingest → query → verify walkthrough. |
| Fix: Docker quickstart published `/ingest` on all host network interfaces | [#49](https://github.com/emmaglorypraise/ObsidianLog/pull/49) | _Done._ Found via manual testing of #48. `/ingest` has no request auth and relies on loopback binding for protection; the compose file published it as `"7080:7080"`, defeating that. Restricted to `"127.0.0.1:7080:7080"`. Trust-boundary decision recorded in ADR-0012. |
| Fix: `obsidianlog init` made redundant OS keychain calls | [#58](https://github.com/emmaglorypraise/ObsidianLog/pull/58) | _Done._ Found via a user testing the published `v0.1.0` release binary: a fresh `init` triggered up to 6 separate macOS keychain authorization prompts — two exact-duplicate existence checks (the keychain-vs-file resolution probe and the idempotency check queried the same account twice) plus a delete that ran unconditionally right before an overwrite that already made it a no-op. `default_key_store()` now returns the already-known existence result instead of discarding it, so callers reuse it rather than re-querying; the pointless pre-store delete is gone. A fresh install now makes 3 keychain calls (4 with the Sia backend chosen), `--force` rotation drops from 6 to 3. No behavior change. Shipped in the [`v0.1.1`](https://github.com/emmaglorypraise/ObsidianLog/releases/tag/v0.1.1) patch release ([#59](https://github.com/emmaglorypraise/ObsidianLog/pull/59)), which also backfilled `CHANGELOG.md` with a proper `[0.1.0]`/`[0.1.1]` split. |
| Month 2 progress report submitted to the Sia Foundation forum | _done_ | Submitted 2026-08-24 (was due 2026-08-25). |

## Month 3 — Launch & Ecosystem Integration (due 2026-09-25)

**Goal:** Ship public developer tooling, integrations, and launch materials.

| Deliverable / task | Pull Request(s) / Commit(s) | Status / Notes |
| --- | --- | --- |
| GitHub Actions reusable workflow | _pending_ | Must be publicly available and forkable. |
| Documentation site | _pending_ | |
| Live end-to-end demo | _pending_ | |
| Indexer topology: hosted-default + bring-your-own-indexer | _in progress_ | Decision recorded in [ADR-0007](../adr/0007-indexer-topology.md). Implementation: make the ingest server's backend selectable (Sia vs local) — **done**, brought forward to Month 2, [#44](https://github.com/emmaglorypraise/ObsidianLog/pull/44) — + operate a hosted indexer (funded wallet, per-app-key quotas, onboarding) — still pending. |
| Example integrations: Grafana + SIEM export workflows | _pending_ | |
| Final MVP report (usage metrics + developer feedback) | _pending_ | |
| Public launch | _pending_ | |

## Success criteria (by end of Month 3)

- Logs flow from Vector to Sia end-to-end, typically within 60 seconds from setup
  completion under normal conditions.
- Archived logs are retrievable via CLI with correct filtering and intact content.
- `obsidianlog verify` hash-chain check passes on all stored chunks.
- `obsidianlog init` completes in under 15 minutes on a clean machine.
- The GitHub Actions demo is publicly available and forkable.
- At least 10 external developers have tested the tool and provided feedback
  (outreach: Sia Discord, developer communities, the Vector community Slack).

## Security practices followed

The Foundation's "Security best practices followed" checklist item is satisfied
by the practices documented in [`SECURITY.md`](../../SECURITY.md): client-side
encryption only, user-controlled keys (OS keychain / `0600` file, no escrow or
transmission), AES-256-GCM authenticated encryption, an append-only +
hash-chained tamper-evident storage model, no intermediary in the storage path,
and `cargo audit` dependency auditing in CI.

## How to update

When a task's PR merges (or, for early work, a commit lands):

1. Fill its row: a link to the PR (`#123`) or commit (short SHA), and any notes
   (difficulties, partial completion, follow-ups).
2. Change `_pending_` / `_in progress_` to `_done_` once complete.


