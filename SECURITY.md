# Security Policy

ObsidianLog handles operational logs and the encryption keys that protect them,
so security reports are taken seriously.

## Reporting a vulnerability

Please report vulnerabilities **privately** using GitHub's
[private vulnerability reporting](https://github.com/emmaglorypraise/ObsidianLog/security/advisories/new)
(Security tab → "Report a vulnerability"). Do not open a public issue.

Include, where possible: affected crate/version, a description, reproduction
steps, and impact. We aim to acknowledge reports within 72 hours.

## Security practices followed

ObsidianLog is a **self-hosted, user-controlled** system. The following practices
are followed to protect users and their data:

- **Client-side encryption only:** No plaintext log data ever leaves the user's
  infrastructure. Encryption occurs before data is written to the Sia network and
  registered via indexd.
- **User-controlled keys:** Key generation happens locally during `obsidianlog
  init`. The encryption key and (if the Sia backend is configured) the Sia
  app key are stored together as one bundled credential in the user's OS
  keychain (via the `keyring` crate on Linux/Windows, and a direct macOS
  Keychain API call on macOS — see ADR-0015), or an explicit local secrets
  file with `0600` permissions if the keychain is genuinely unreachable. No
  key escrow, no key transmission.
- **Authenticated encryption:** AES-256-GCM provides both confidentiality and
  ciphertext integrity. Tampered ciphertext is rejected at decryption time.
- **Append-only storage model:** Chunks are write-once. The storage model is
  designed to prevent in-place modification, relying on append-only writes
  combined with indexd-coordinated retrieval and SHA-256 hash chaining for tamper
  evidence (`obsidianlog verify`). This provides strong tamper-evidence without
  requiring a blockchain.
- **No intermediary in the storage path:** ObsidianLog connects to the user's Sia
  node with indexd support for indexing and retrieval coordination, without any
  centralized intermediary. There is no ObsidianLog-operated proxy, gateway, or
  relay for the self-hosted MVP. The builder has zero access to user data.
- **Dependency audit:** The Rust dependency tree is audited with `cargo audit` in
  CI (the `audit` job in `.github/workflows/ci.yml`). Critical dependencies
  (`zstd`, `aes-gcm`, `sha2`) are maintained by the RustCrypto and zstd-rs
  communities with strong security track records.

## Out of scope

The security of the user's own indexd deployment, Sia host selection, and the
user's operational key-management practices are outside ObsidianLog's control.
