# 0017 — `verify` reconciles the backend's contents with the manifest

- Status: Accepted
- Date: 2026-11-24

## Context

`obsidianlog verify` (ADR-0003) walks each service's chain using the chunk
list in the manifest. It never asks the backend what is actually stored, so a
chunk the manifest doesn't list is invisible to it. That happens when:

- the manifest is rolled back or restored from an older backup while newer
  chunks stay in storage;
- a manifest write fails after its chunks were stored (ADR-0013, ADR-0016);
- a bug, or a second writer, forks a chain, so two stored chunks claim the
  same sequence number (the Sia listing bugs fixed in #124 and #125 did this);
- the manifest loses a service entirely.

In each case `verify` reported "chain intact". The Month 3 final review
asked for `verify` to list the backend, reconcile it against the manifest,
and fail on a duplicate nonce.

## Decision

`verify` keeps every existing check and adds three:

1. **Per service, reconcile the listing with the manifest.** After the chain
   checks pass, `verify_service` calls `StorageBackend::list_chunks(service,
   None)` and fails if:
   - more than one stored chunk claims the same sequence number
     (`ArchiveMismatch::DuplicateSequence`, a forked chain), or
   - a stored chunk isn't in the manifest's chunk list
     (`ArchiveMismatch::Unlisted`).

   If the manifest doesn't know the service at all but chunks are stored for
   it, `verify_service` fails with `ArchiveMismatch::UnknownService` instead of
   reporting zero chunks.
2. **Across the archive, find services the manifest has lost.** A new
   `StorageBackend::list_services` method returns every service with at least
   one stored chunk. `verify_all` adds any service missing from the manifest
   to its results, as a failure.
3. **Across the archive, fail on a reused (key, nonce) pair.** `verify_all`
   collects every verified chunk's `(key_salt, nonce)` and fails both services
   involved (`ArchiveMismatch::NonceReuse`) if two chunks share one. The key a
   chunk is sealed under is identified by its salt (ADR-0016); chunks written
   before ADR-0016 have no salt and share the master key, so their nonces alone
   must be unique.

Mismatches use a new `ArchiveMismatch` error type rather than new
`ChainBreakKind` variants. `ChainBreakKind` is a public, exhaustive enum in a
published crate, so adding variants could break downstream code that matches
on it. `ArchiveMismatch` is `#[non_exhaustive]`, and both convert into
`obsidianlog_core::Error::ChainIntegrity`, so the CLI's output and exit codes
work unchanged.

`list_services` has a default implementation returning an empty list, so
backends outside this repository keep compiling. Such a backend still gets the
per-service reconciliation, but not the lost-service check. `LocalBackend`,
`SiaBackend` and `AnyBackend` all implement it.

## Consequences

- A rolled-back manifest is now caught whenever the newer chunks are still in
  storage. Someone who deletes those chunks **and** rewrites the manifest to
  match is still not caught by `verify` alone; publishing chain heads outside
  the archive is the follow-up for that case.
- `verify` does more I/O: one extra listing per service, plus one to list
  services. On `LocalBackend` this is a directory scan. On `SiaBackend` each
  listing reads every object event, so `verify` costs roughly one full listing
  per service. That is acceptable for an on-demand command; a single shared
  listing is a possible later optimisation.
- On `LocalBackend` the listing comes from index files. A chunk file whose
  index file is missing is not listed, so it isn't reported as unlisted. The
  existing missing-chunk check still catches the reverse (an index without
  its chunk).
- The nonce check only sees chunks `verify` fetched, that is, chunks in the
  manifest. An unlisted chunk already fails reconciliation, so it isn't
  fetched for the nonce check.
- Existing archives need no migration. An archive that previously verified
  clean only fails now if storage really holds something the manifest
  doesn't describe.
