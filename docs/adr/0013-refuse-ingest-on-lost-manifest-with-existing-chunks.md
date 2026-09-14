# 0013 — Refuse to ingest when a service has chunks but no manifest entry

- Status: Accepted
- Date: 2026-09-09

## Context

ADR-0009 replaced the hashed-name nonce discriminator with a manifest-assigned
`service_id`, closing the cross-service nonce collision. Its Consequences
section flagged a second, independent gap it did **not** close:

> if `manifest.json` is lost or restored stale while a service's chunk files
> remain, `ArchiveEngine` currently treats the missing manifest as a fresh
> start (`next_sequence = 0`) and will reuse nonces already used under the
> same key for that service.

Concretely: `ArchiveEngine::read_manifest_or_default` treats a missing
manifest (`Error::NotFound`) as a brand-new archive (`Manifest::new()`).
`ensure_service_ids` then sees every service as never-before-seen and assigns
each a fresh `service_id` starting at `next_sequence = 0`. If a service's
chunk files are still sitting on the backend from before the manifest was
lost, the next chunk gets a nonce already used under the same key for that
service — a full AES-256-GCM (key, nonce) reuse (ADR-0002), with the same
consequences as the collision ADR-0009 fixed: the reused keystream leaks the
XOR of both plaintexts, and exposes the GHASH subkey, letting an attacker
forge authentication tags.

Unlike ADR-0009's gap, this needs no attacker, no adversarial service name,
and no second service — a lost or stale manifest is sufficient on its own,
which ADR-0009 called "arguably the more operationally realistic of the two."

A regression test, `crates/obsidianlog-store/tests/manifest_loss_nonce_reuse.rs`,
reproduces it directly: ingest a batch, delete `manifest.json` while the
chunk file remains, ingest again for the same service, and confirm the
second ingest does not silently succeed.

## Decision

Before `ensure_service_ids` registers a service as new (i.e. before it hands
out a fresh `service_id` and implicitly starts that service's `next_sequence`
at 0), it now checks the backend for chunks already recorded for that
service via `StorageBackend::list_chunks(service, None)`. If any are found,
`ingest_batch` fails with a `Backend` error instead of proceeding — the
manifest is missing an entry it should not be missing, and the safe response
is to refuse and require operator intervention, not silently reuse a nonce.

This check runs only for services not already known to the manifest, under
the existing `manifest_lock` (ADR-0009), so it adds no cost to the common
case (every touched service already registered) and a single extra
`list_chunks` call only when a batch introduces a service the engine has
never seen before — the same cost shape ADR-0009 already established for
`service_id` assignment.

## Consequences

- Closes the residual gap ADR-0009 left open: a lost or stale manifest can no
  longer cause a silent nonce reuse for a service whose chunk files survive
  it. The failure mode changes from "silent data-confidentiality and
  tamper-evidence break" to "loud refusal to ingest."
- Does not provide recovery. An operator who hits this must reconstruct or
  restore the manifest (or migrate the service to a new name/bucket) before
  ingest for that service can resume; this ADR only prevents the unsafe
  automatic path, it doesn't replace one.
- Does not protect a service whose chunk files were lost or deleted along
  with the manifest — `list_chunks` finds nothing to refuse on, and the
  service is (correctly, since there's genuinely nothing left to protect)
  treated as new. This decision is about detecting *inconsistency* between
  the manifest and the backend, not about backing up either one.
- A batch that touches both an already-known service and a service hitting
  this guard fails as a whole (`ingest_batch` returns before any upload for
  that batch), consistent with the batch's existing all-or-nothing manifest
  update.
