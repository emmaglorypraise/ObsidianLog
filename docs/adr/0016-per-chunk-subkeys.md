# 0016 — Per-chunk subkeys so nonce safety doesn't depend on the manifest

- Status: Accepted
- Date: 2026-10-16

## Context

ADR-0002 derives each chunk's AES-256-GCM nonce from its service's
`next_sequence` counter in `manifest.json`, and calls that counter
"safety-critical state": resuming from a stale counter reuses a nonce. ADR-0009
made the cross-service part of the nonce unconditional, ADR-0013 refuses
ingest when a service has chunks but no manifest entry, and ADR-0014 stops two
local processes writing the same directory. Each closed one way for the
counter to go wrong, but the design still rests on the persisted counter never
falling behind what has actually been used.

The Month 3 reviewer pointed out it still can. `ArchiveEngine::ingest_batch`
encrypts and uploads a batch's chunks **first** and advances the counter
**afterwards**, in a separate manifest write. Reproduced against `main`:

1. **Failed manifest write, then the next batch.** The chunk upload succeeds,
   the manifest write fails, the request errors. The counter was never
   advanced, so the next batch (Vector's normal retry, or simply the next logs)
   is sealed with the same sequence, and so the same `(key, nonce)`, as the
   orphaned chunk still sitting on the backend. A crash between the upload and
   the manifest write does the same.
2. **Whole data directory restored from an older backup.** The restored
   counter is behind; the next chunk reuses a nonce whose earlier ciphertext
   may survive elsewhere (a newer backup, a Sia upload).
3. **Only `manifest.json` restored from an older backup.** The service still
   has a manifest entry, just a stale one, so ADR-0013's guard doesn't fire;
   the next chunk reuses a nonce of a chunk that is still on disk.

Two `serve` instances writing the same Sia bucket with the same credential
would collide the same way (ADR-0014's lock covers only the local backend).
None of these needs an attacker, and the consequence of a single reuse is the
full GCM break described in ADR-0002: the XOR of the two plaintexts leaks and
the GHASH subkey is exposed, letting tags be forged.

Patching each path individually (reserve the counter before encrypting, scan
the backend for chunks beyond the counter on startup) would close 1 and 3 but
not 2, where the evidence of the used nonces is gone from the restored
directory, and would leave nonce safety resting on every future code path and
operational procedure getting the counter right.

## Decision

Seal each chunk under its **own subkey**, so the uniqueness of the effective
`(key, nonce)` pair no longer depends on any persisted state.

- For every chunk, generate a fresh 256-bit salt from the OS CSPRNG
  (`encrypt::generate_salt`).
- Derive the chunk's AES-256 key with HKDF-SHA256: input keying material = the
  master key, salt = the chunk salt, info = `obsidianlog/chunk-key/v1`
  (`EncryptionKey::derive_chunk_key`).
- Encrypt with AES-256-GCM under that subkey, with the nonce still derived from
  `(service_id, sequence)` exactly as before (ADR-0002, ADR-0009), kept as
  defense in depth.
- Store the salt in the chunk header as `key_salt`. It is part of
  `ChunkHeader::canonical_bytes`, so the hash chain covers it and a tampered
  salt is detected by `verify` as well as by GCM.

**Backward compatibility.** `key_salt` is optional. Chunks written before this
ADR have no salt: their header JSON omits the field, their canonical bytes (and
so their chain hashes) are unchanged, and they decrypt directly under the
master key as before. Existing archives need no migration, and chains that mix
old and new chunks verify.

The compatibility is one-way. Binaries from before this ADR (v0.2.1 and
earlier) ignore the unknown `key_salt` field and try to decrypt new chunks
under the master key, which fails GCM authentication. Once a new binary has
written to an archive, it can't be read by an older one.

This is the alternative ADR-0002 considered and deferred ("a clean future
option"), adopted now with a random salt rather than a salt derived from
`(service, sequence)`: a derived salt would repeat exactly when the sequence
repeats, which is the failure being fixed.

## Consequences

- All three reproduced paths, the crash-mid-batch variant, and the
  two-writers-on-Sia variant can no longer reuse a `(key, nonce)` pair. Two
  chunks share a key only if their random 256-bit salts collide: about 2^-128
  even after 2^64 chunks. Covered by
  `crates/obsidianlog-store/tests/counter_rollback_nonce_reuse.rs`.
- This changes the CLAUDE.md invariant "derive deterministically from a
  per-service monotonic counter". The concern behind that rule was the
  birthday bound of *random 96-bit nonces* (unsafe past ~2^32 messages per
  key). A 256-bit salt into HKDF is far outside that regime; the nonce itself
  is still counter-derived.
- Each new chunk header grows by 32 bytes (plus its JSON encoding), and each
  chunk encryption/decryption costs one HKDF-SHA256 call, negligible next to
  zstd and the network.
- The manifest counter is no longer safety-critical for confidentiality or
  tag integrity. It still matters for data completeness: a chunk written past
  a rolled-back counter is left orphaned (the manifest doesn't reference it,
  so `query` and `verify` never see it), and a repeat in the same time window
  overwrites the earlier chunk file, since the path includes the sequence
  (ADR-0010). ADR-0013's refusal stays in place for the lost-manifest case.
  Detecting and recovering orphaned chunks is a separate follow-up.
- The impact of a master-key compromise is unchanged: the master key still decrypts every
  chunk, since every subkey derives from it.
