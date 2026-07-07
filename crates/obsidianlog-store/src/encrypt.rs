//! Client-side AES-256-GCM authenticated encryption.
//!
//! Each chunk is sealed with AES-256-GCM before it leaves the user's
//! infrastructure. The GCM tag authenticates the ciphertext, so any tampering —
//! a flipped bit, the wrong key, or the wrong nonce — is rejected at decryption
//! time rather than silently returning garbage.
//!
//! ## Nonce invariant (critical)
//!
//! A `(key, nonce)` pair MUST NEVER be reused. A single reuse under one key is
//! catastrophic for GCM: it leaks the XOR of the two plaintexts and lets an
//! attacker forge the authentication tag. This module therefore **never
//! generates a nonce** — the caller supplies it, derived deterministically from
//! the chunk's per-service monotonic sequence number (the counter lives in the
//! `Manifest`). That counter is the single source of nonce uniqueness. See
//! ADR-0002.

use aes_gcm::aead::{Aead, Nonce};
use aes_gcm::{Aes256Gcm, Key, KeyInit};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::{Error, Result};

/// AES-256 key length, in bytes.
pub const KEY_LEN: usize = 32;
/// AES-GCM nonce length, in bytes (96 bits).
pub const NONCE_LEN: usize = 12;
/// AES-GCM authentication tag length, in bytes, appended to each ciphertext.
pub const TAG_LEN: usize = 16;

/// A 256-bit AES key.
///
/// The key bytes are zeroized when the value is dropped, so they do not linger
/// in freed memory. The key material is never logged or `Display`ed, and `Debug`
/// is redacted.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct EncryptionKey([u8; KEY_LEN]);

impl EncryptionKey {
    /// Wrap raw key bytes. The caller is responsible for generating them from a
    /// CSPRNG (see the CLI key store) and keeping them secret.
    pub fn new(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }
}

// Redacted Debug so key material can never leak into logs or error output.
// There is intentionally no `Display` impl.
impl std::fmt::Debug for EncryptionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EncryptionKey(<redacted>)")
    }
}

/// Derive the deterministic 96-bit nonce for a chunk from its `service` and
/// per-service `sequence` number.
///
/// Layout: the first 4 bytes are a service discriminator (`SHA-256(service)[..4]`)
/// and the last 8 bytes are the big-endian sequence. Within one service the
/// discriminator is constant and the monotonic `sequence` is unique, so a
/// `(key, nonce)` pair is **never reused for that service** — the guarantee GCM
/// depends on (see ADR-0002). Across services the 32-bit discriminator makes a
/// collision negligible for realistic service counts; a manifest-assigned unique
/// service id would make cross-service uniqueness total.
pub fn derive_nonce(service: &str, sequence: u64) -> [u8; NONCE_LEN] {
    let digest = Sha256::digest(service.as_bytes());
    let mut nonce = [0u8; NONCE_LEN];
    nonce[..4].copy_from_slice(&digest[..4]);
    nonce[4..].copy_from_slice(&sequence.to_be_bytes());
    nonce
}

/// Encrypt `plaintext` with AES-256-GCM under `key` and `nonce`.
///
/// Returns `ciphertext || tag` (the 16-byte GCM tag is appended). `nonce` MUST
/// be unique for `key` — see the module-level nonce invariant.
pub fn encrypt_chunk(
    key: &EncryptionKey,
    nonce: [u8; NONCE_LEN],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(key.0));
    let nonce = Nonce::<Aes256Gcm>::from(nonce);
    cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| Error::Crypto("AES-256-GCM encryption failed".into()))
}

/// Authenticated-decrypt `ciphertext` (`ciphertext || tag`) with `key` and `nonce`.
///
/// Fails with [`Error::Crypto`] if the tag does not verify — i.e. the data was
/// tampered with, or the wrong key or nonce was supplied. No plaintext is
/// returned in that case.
pub fn decrypt_chunk(
    key: &EncryptionKey,
    nonce: [u8; NONCE_LEN],
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(&Key::<Aes256Gcm>::from(key.0));
    let nonce = Nonce::<Aes256Gcm>::from(nonce);
    cipher
        .decrypt(&nonce, ciphertext)
        .map_err(|_| Error::Crypto("AES-256-GCM authentication failed".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONCE: [u8; NONCE_LEN] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

    fn key_a() -> EncryptionKey {
        EncryptionKey::new([0x11; KEY_LEN])
    }

    fn key_b() -> EncryptionKey {
        EncryptionKey::new([0x22; KEY_LEN])
    }

    #[test]
    fn encrypt_then_decrypt_round_trips() {
        let key = key_a();
        let plaintext: &[u8] = b"the quick brown fox jumps over the lazy dog";

        let ciphertext = encrypt_chunk(&key, NONCE, plaintext).expect("encrypt");
        assert_ne!(
            ciphertext.as_slice(),
            plaintext,
            "ciphertext must differ from plaintext"
        );
        assert_eq!(
            ciphertext.len(),
            plaintext.len() + TAG_LEN,
            "the GCM tag is appended to the ciphertext"
        );

        let recovered = decrypt_chunk(&key, NONCE, &ciphertext).expect("decrypt");
        assert_eq!(recovered.as_slice(), plaintext);
    }

    #[test]
    fn flipping_a_ciphertext_byte_fails_authentication() {
        let key = key_a();
        let mut ciphertext = encrypt_chunk(&key, NONCE, b"sensitive log line").expect("encrypt");

        ciphertext[0] ^= 0x01; // tamper with a single bit
        assert!(
            decrypt_chunk(&key, NONCE, &ciphertext).is_err(),
            "authentication must catch tampering"
        );
    }

    #[test]
    fn wrong_nonce_fails() {
        let key = key_a();
        let ciphertext = encrypt_chunk(&key, NONCE, b"payload").expect("encrypt");

        let mut wrong = NONCE;
        wrong[0] ^= 0xFF;
        assert!(decrypt_chunk(&key, wrong, &ciphertext).is_err());
    }

    #[test]
    fn wrong_key_fails() {
        let ciphertext = encrypt_chunk(&key_a(), NONCE, b"payload").expect("encrypt");
        assert!(decrypt_chunk(&key_b(), NONCE, &ciphertext).is_err());
    }

    #[test]
    fn derive_nonce_is_deterministic_unique_per_sequence() {
        // Deterministic.
        assert_eq!(derive_nonce("api", 7), derive_nonce("api", 7));
        // Unique per sequence within a service.
        assert_ne!(derive_nonce("api", 0), derive_nonce("api", 1));
        // Different services get different nonces at the same sequence.
        assert_ne!(derive_nonce("api", 0), derive_nonce("web", 0));
        // The sequence occupies the low 8 bytes.
        assert_eq!(derive_nonce("api", 42)[4..], 42u64.to_be_bytes());
    }
}
