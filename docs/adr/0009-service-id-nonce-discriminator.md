# 0009 — Manifest-assigned service ids as the nonce discriminator

- Status: Accepted
- Date: 2026-08-04

## Context

ADR-0002 establishes that AES-256-GCM's one non-negotiable rule — a `(key,
nonce)` pair must never repeat — is met by deriving each chunk's 96-bit nonce
deterministically from a monotonic per-service sequence counter, and states
that "every nonce is unique by construction — there is zero collision
probability." That claim covered uniqueness *within* one service correctly,
but was wrong about uniqueness *across* services: it didn't account for how
the implementation actually separated services within the nonce space.

The implementation (`derive_nonce`) built the nonce as
`SHA-256(service)[..4] || sequence(8 bytes BE)` — a 32-bit truncated hash of
the service *name* as the cross-service discriminator, with every service
encrypted under the **same** `ArchiveEngine` key. Cross-service uniqueness
therefore rested entirely on that 32-bit hash never colliding between two
service names. This is:

- **Not construction-guaranteed** — only probabilistically unlikely, and only
  under a non-adversarial name distribution (~50% collision risk near 2^16
  distinct service names, by the birthday bound).
- **Not adversarially safe** — the `service` field is copied verbatim from
  ingested log records with no validation, so it is attacker-influenceable. A
  32-bit space is a cheap, deterministic, ~2^32 offline search (seconds on
  commodity hardware), not a meaningful barrier.
- **Reachable without an attacker at all** — a service's `next_sequence`
  counter resets to a nonce already used under the same key if the manifest is
  ever lost or restored from a stale backup while the service's chunk files
  remain (a same-service failure mode, independent of this ADR's fix — see
  Consequences).

A Month-1 milestone reviewer flagged this and linked a proof-of-concept
demonstrating it: [`nonce_reuse_poc.rs`][poc]. Reproduced locally against this
codebase, it confirmed — through the real `ArchiveEngine`/`LocalBackend`
pipeline — that an attacker who can post logs under a chosen colliding service
name recovers another service's plaintext log content from stored ciphertext
alone, with no access to the encryption key. Reused `(key, nonce)` under GCM
is a full break of both confidentiality (the reused keystream leaks the XOR of
both plaintexts) and the tamper-evidence guarantee (the reused keystream also
exposes the GHASH subkey, enabling forged authentication tags) — undermining
the two core promises of the project for the affected chains. That PoC is
superseded here by `crates/obsidianlog-store/tests/nonce_uniqueness.rs`, a
regression test proving the same scenario (two names that collided under the
old scheme) no longer works.

[poc]: https://github.com/Alrighttt/ObsidianLog/blob/main/crates/obsidianlog-store/tests/nonce_reuse_poc.rs

## Decision

Replace the hashed-name discriminator with a **manifest-assigned unique
`service_id: u32`**, handed out once per service, at first use, from a new
authoritative monotonic counter, `Manifest::next_service_id`. The nonce is now
`service_id.to_be_bytes() || sequence.to_be_bytes()` — no hashing involved.
Because the id comes from a counter the manifest itself owns (never derived
from attacker-supplied data), two services can never be assigned the same
discriminator: cross-service nonce uniqueness is now **unconditional**, not
probabilistic, matching the guarantee ADR-0002 already had for the
within-service case.

`ArchiveEngine::ingest_batch` ensures every batch-touched service has a
durable `service_id` before deriving any nonce (`ensure_service_ids`). The
common case — every touched service already known — costs nothing extra: a
single unlocked manifest read, exactly as before this change. Only when a
batch introduces a genuinely new service does it take the `manifest_lock` to
read-modify-write the registration, durably, before any chunk for that batch
is encrypted. This is a one-time cost per service, not an ongoing one; the
existing per-service locks (ADR-0003) already prevent two batches for the
*same* new service racing on this, and the `manifest_lock` round-trip is what
prevents two batches introducing two *different* new services concurrently
from being assigned the same id.

### Manifest format is a breaking change

`Manifest.next_service_id` and `ManifestServiceChain.service_id` are both
required fields (no `#[serde(default)]`), and `MANIFEST_VERSION` is bumped
from 1 to 2. A v1 manifest **fails to deserialize** against this version — it
does not silently reinterpret with a missing-field default. A shared default
(e.g. every pre-existing chain silently defaulting to `service_id = 0`) would
reintroduce exactly the vulnerability this ADR fixes, for every deployment
with two or more pre-existing services. Loudly refusing to load is the safe
failure mode; silently misassigning colliding ids is not. No automatic
migration tool is provided in this change, since no real deployments exist
yet (the project has not had a public launch — see
`docs/grant/PROGRESS.md`).

Already-archived chunk data is unaffected regardless: each chunk's nonce is
read from its own stored header at decrypt time, never recomputed via
`derive_nonce`. Only the *manifest* format is breaking, not the encrypted
chunk data itself.

## Consequences

- Cross-service nonce uniqueness no longer depends on a 32-bit hash, a
  birthday bound, or trusting the service name — it's structural, matching the
  bar ADR-0002 already set for the within-service case. ADR-0002's "zero
  collision probability" claim is corrected by this ADR to note it only ever
  held within one service.
- `ingest_batch` pays one extra manifest round-trip (a read, and a write only
  when new services are involved) — a one-time cost per service, not a
  steady-state one.
- The manifest format bump means any existing v1 manifest must be regenerated;
  there is no in-place migration. Acceptable now (no production deployments);
  would need revisiting before a real migration is ever required.
- **Not fixed by this ADR, and still worth tracking:** the same-service
  sequence-reset trigger described above — if `manifest.json` is lost or
  restored stale while a service's chunk files remain, `ArchiveEngine`
  currently treats the missing manifest as a fresh start (`next_sequence =
  0`) and will reuse nonces already used under the same key for that service.
  This is independent of the cross-service fix here (it doesn't need a second
  service, an attacker, or any collision search) and is arguably the more
  operationally realistic trigger of the two. A reasonable follow-up: before
  archiving under a "fresh" manifest, check the backend (e.g.
  `list_chunks`) for chunk files that already exist for a service the
  manifest doesn't know about, and refuse rather than silently restart the
  counter.
