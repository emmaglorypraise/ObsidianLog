# 0011 — The standalone ingest binary requires an explicit encryption key

- Status: Accepted
- Date: 2026-08-06

## Context

Two independent binaries can run the HTTP ingest server:

- `obsidianlog serve` (the CLI) always loads the real, CSPRNG-generated key
  from the OS keychain (or a `0600` secrets file) before starting, and errors
  out with "run `obsidianlog init` first" if none exists.
- The standalone `obsidianlog-ingest` binary — documented as existing "for
  running the server directly" — never touched the keystore. Its `Config`
  struct deliberately keeps `encryption_key` out of the plaintext TOML config
  (`#[serde(skip)]`), which is correct, but nothing filled it in from anywhere
  else either: it silently defaulted to `EncryptionKey::new([0u8; 32])`, a
  hardcoded, publicly-known key. `serve()` only printed a `stderr` warning
  when this placeholder was in use; it did not refuse to start.

The result: running `obsidianlog-ingest config.toml` — a real, supported
usage path, not a misuse — archived every log encrypted under a key any
attacker already knows, defeating confidentiality entirely, with only an
easy-to-miss warning line (e.g. lost in a container's combined log stream) as
the only signal anything was wrong.

### Why not auto-generate a key the same way `obsidianlog init` does

The obvious alternative — have the standalone binary transparently generate
and persist a key on first run, mirroring the CLI — was considered and
rejected. This binary's realistic deployment target is a **container**
(Docker/Kubernetes), which is typically ephemeral: without an explicitly
mounted, persistent volume for the key file, every restart/redeploy would
silently generate a *new* key, permanently orphaning everything encrypted
under the previous one. That failure mode is silent, rolling, and worse than
the placeholder-key bug it would replace — the same class of "silent data
loss via a well-intentioned default" this project has already hit twice this
month (see ADR-0009, ADR-0010). A daemon meant to run non-interactively
should receive its secret from outside the process, not invent one on its
own and hope local disk persists forever.

## Decision

1. `serve()` (`obsidianlog-ingest`) now **refuses to start** — a hard
   `Error::Config`, not a warning — when `config.encryption_key` is the
   all-zero placeholder. This is the actual security backstop, and it
   protects every caller of `serve()`, not just the standalone binary's
   `main()`.
2. The standalone binary's `main()` supplies a real key via
   `config::encryption_key_from_env()`, checked in this order:
   - `OBSIDIANLOG_ENCRYPTION_KEY_FILE` — a path to a file holding a
     64-character hex key. Preferred: this is the Docker/Postgres `_FILE`
     convention, keeping the key out of the process environment (and
     therefore out of `/proc/<pid>/environ` and process listings), and maps
     directly onto a mounted Docker secret / Kubernetes `Secret` / Compose
     `secrets:` entry with no extra glue.
   - `OBSIDIANLOG_ENCRYPTION_KEY` — the hex key itself, for simpler/dev
     setups.
   - Neither set, or the value isn't a valid key → fails fast with a message
     naming both options and pointing at `obsidianlog serve` as the
     keychain-backed alternative.

The CLI's `obsidianlog serve` is unaffected: it never calls `serve()` with a
placeholder key (it builds the engine directly with a real, keychain-loaded
key), so this change is invisible to CLI users.

## Consequences

- Closes the silent-confidentiality-failure path completely: the standalone
  binary can no longer run with a known key, ever.
- **Breaking for existing standalone-binary users**: anyone currently running
  `obsidianlog-ingest` with no key configured (i.e., already broken, just
  silently) must now set one of the two env vars, or the process exits
  immediately. There is no way to opt back into the old silent-placeholder
  behavior — that was the point.
- No change to the plaintext-config-never-holds-key-material invariant: the
  key still never appears in `config.toml`.
- Sets the intended pattern for future container/deployment docs (Docker
  Compose, Kubernetes manifests): mount the key as a file and point
  `OBSIDIANLOG_ENCRYPTION_KEY_FILE` at it, rather than baking it into an env
  var or, worse, an image layer.
