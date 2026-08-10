# 0012 — `/ingest` relies on network isolation, not request authentication

- Status: Accepted
- Date: 2026-08-10

## Context

`POST /ingest` (`crates/obsidianlog-ingest/src/server.rs`) accepts a batch
from any caller that can reach the port — there is no API key, token, or
signature check. `service` is taken verbatim from the posted JSON
(`obsidianlog-store/src/parse.rs`) with no allow-list, so any caller can
submit a batch claiming to be any existing service. It gets chained into
that service's real hash chain: `chain.rs` only verifies internal
consistency (link-hash, sequence), never who submitted a chunk.
Tamper-evidence protects against post-hoc alteration of archived data, not
against forged data being chained in live.

This was flagged as a MEDIUM finding in security review. The stated
mitigation — `bind` defaults to `127.0.0.1:7080` — held for the standalone
binary and CLI, but the Docker Compose quickstart
(`docker/docker-compose.yml`) was silently defeating it: the ingest
container's `CMD` overrides `bind` to `0.0.0.0:7080` (required so the
published port is reachable at all — see `docker/Dockerfile`), and the
compose file published that port as `"7080:7080"`, i.e. on every host
network interface rather than just loopback. Anyone on the same network as
the Docker host could reach `/ingest` with no auth, even though the
non-Docker default was safe.

### Why not add request authentication

A shared-secret or per-service token check was considered. Rejected for
now:

- ObsidianLog's actual deployment shape, per the README, is Vector running
  as a co-located sidecar POSTing to a same-host `obsidianlog` — the same
  trust model Vector's own HTTP sink uses elsewhere. There is no current
  use case where Vector and the ingest server run on different hosts or
  across a shared/untrusted network.
- The CLI doesn't wire up `SiaBackend` yet (Month-3 work, ADR-0007), so
  there's no multi-host or multi-tenant ingest deployment to protect
  against today.
- Real auth is nontrivial scope done properly: secret shape (global vs.
  per-service), delivery (env var vs. keychain, mirroring
  ADR-0011's `_FILE`/`_KEY` convention), rotation, and a breaking wire-format
  change — all speculative work for a threat model nobody is currently
  exposed to. Building it now would be designing for a hypothetical future
  requirement instead of an actual one.

## Decision

We will treat `/ingest` as a **trusted-network-only** endpoint by
containment, not by request authentication:

1. `docker-compose.yml` publishes the ingest port as
   `"127.0.0.1:7080:7080"`, restoring loopback-only reachability — the same
   pattern already used for `indexd`'s admin port in the same file.
2. The README documents this trust boundary explicitly: `/ingest` must
   never be exposed beyond a private network, and `bind`/the Docker port
   mapping must stay on loopback.
3. We do **not** add shared-secret or token auth at this time.

## Consequences

- Closes the concrete regression Docker introduced (port published on all
  interfaces); restores the loopback assumption the original bind default
  relies on.
- Does not close the underlying gap: any process with access to the host's
  loopback interface (another local container, another user account, a
  compromised local process) can still forge batches into any service's
  chain. This is accepted as in-scope for "trusted host," not for "trusted
  network."
- If a future milestone introduces a legitimate need for Vector and
  `obsidianlog` to run on different hosts, or a multi-tenant/shared ingest
  deployment, this decision must be revisited — that's the point at which
  the shared-secret design questions above (secret shape, delivery,
  rotation) become real tradeoffs instead of speculative ones. A future ADR
  superseding this one should make that call then, informed by the actual
  deployment shape at the time.
