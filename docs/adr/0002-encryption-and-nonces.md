# 0002 — Encryption and nonces

- Status: Accepted
- Date: 2026-06-29

> **Amended by [ADR-0009](0009-service-id-nonce-discriminator.md) (2026-08-04).**
> The "zero collision probability" claim below held only for uniqueness
> *within* one service, not *across* services — the original cross-service
> discriminator (`SHA-256(service)[..4]`) was a probabilistic 32-bit hash of
> the service name, not a construction guarantee, and the name is
> attacker-influenceable. ADR-0009 replaces it with a manifest-assigned
> `service_id`, making cross-service uniqueness unconditional too. Everything
> else below (AES-256-GCM, per-service counter-based nonces, the rejected HKDF
> alternative) is unchanged.

## Context

Chunks are encrypted client-side, before any data leaves the user's
infrastructure, and must be both confidential and tamper-evident: a reader must
be able to detect if a stored chunk was modified. We also need a nonce strategy
that is safe for an archive that grows without bound, where the same key may
protect billions of chunks over years.

## Decision

### AES-256-GCM

Encrypt each chunk with **AES-256-GCM**, an authenticated encryption with
associated data (AEAD) construction. GCM gives us confidentiality *and* a
16-byte authentication tag in one pass: any modification to the ciphertext (or
use of the wrong key or nonce) fails authentication at decryption time instead of
returning corrupt plaintext. AES-256 is ubiquitous, hardware-accelerated
(AES-NI), and well-reviewed. The tag is appended to the ciphertext
(`ciphertext || tag`); the 96-bit nonce is stored in the chunk header, not in the
ciphertext body.

### Deterministic, counter-based nonces

GCM's security depends on an absolute invariant: a **(key, nonce) pair must never
repeat**. Rather than sample nonces randomly, ObsidianLog **derives each nonce
deterministically from the chunk's per-service monotonic sequence number**. Each
service has its own append-only chain (ADR-0003); the chain's *next sequence*
counter lives in the [`Manifest`] (`ManifestServiceChain::next_sequence`). When a
chunk is sealed, its sequence number is encoded into the 96-bit nonce.

Because the counter is monotonic and has a single writer per service (writes
within a service are serialized), **every nonce is unique by construction** —
there is zero collision probability. The `Manifest` is therefore the single
source of nonce uniqueness; recovering it (and the per-service counter) on
startup is what guarantees the next nonce has never been used.

The `encrypt` module never generates a nonce itself: the nonce is passed in by
the pipeline. This keeps the uniqueness guarantee in one place (the counter)
rather than spread across call sites.

### Alternative considered: HKDF-derived per-chunk subkey with a fixed nonce

We considered deriving a **fresh per-chunk subkey** via HKDF (e.g.
`subkey = HKDF(master_key, service || sequence)`) and then encrypting with a
**fixed nonce**. This also avoids (key, nonce) reuse, since each chunk uses a
distinct key. We rejected it for the MVP because it adds a key-derivation step
and a second primitive (HKDF) on the hot path for no additional safety over a
correctly managed counter, and it makes key handling more complex (a key per
chunk rather than one per service). The counter-based nonce achieves the same
uniqueness guarantee with less machinery. The HKDF approach remains a clean
future option if we ever want to bound the amount of data under a single key.

## Consequences

- Nonce uniqueness is structural, not probabilistic — we never rely on the
  birthday bound of random 96-bit nonces, which becomes unsafe at archival scale.
- The per-service counter in the `Manifest` is **safety-critical state**: it must
  be persisted durably and recovered before issuing the next nonce. Resuming from
  a stale counter would reuse a nonce. (This is one reason backend writes must be
  durable before returning `Ok` — see ADR-0004/0005.)
- **Consequence of nonce reuse (why this matters):** reusing a (key, nonce) pair
  in GCM is catastrophic — it leaks the XOR of the two plaintexts (breaking
  confidentiality) and exposes the GCM authentication subkey, letting an attacker
  forge valid tags (breaking integrity). The whole tamper-evidence guarantee
  collapses. This is why the nonce is derived from a counter, not chosen at the
  call site.
- The nonce is stored per chunk (in the header), so decryption needs the header
  but no shared counter state.

[`Manifest`]: ../../crates/obsidianlog-core/src/manifest.rs
