//! User-controlled encryption key storage.
//!
//! Keys are generated locally during `obsidianlog init` and never transmitted.
//! They are stored in the OS keychain via the `keyring` crate (Linux/macOS/
//! Windows), or an explicit local secrets file with `0600` permissions. This
//! module is the only place keys are read or written.

use anyhow::Result;

use obsidianlog_store::encrypt::EncryptionKey;

/// Service name used to namespace ObsidianLog entries in the OS keychain.
pub const KEYRING_SERVICE: &str = "obsidianlog";

/// Where the encryption key is persisted.
#[derive(Debug, Clone)]
pub enum KeySource {
    /// The OS keychain (default, recommended).
    Keyring,
    /// An explicit local secrets file (must be `0600`).
    File(std::path::PathBuf),
}

/// Generate a fresh AES-256 key and persist it to `source`.
///
/// TODO(impl): CSPRNG-generate 32 bytes, store via keyring or a `0600` file, and
/// refuse to overwrite an existing key unless explicitly forced.
pub fn generate(source: &KeySource) -> Result<EncryptionKey> {
    let _ = source;
    anyhow::bail!("keystore::generate is not yet implemented")
}

/// Load the encryption key from `source`.
///
/// TODO(impl): read the key from the keyring or secrets file (verifying file
/// permissions are `0600`).
pub fn load(source: &KeySource) -> Result<EncryptionKey> {
    let _ = source;
    anyhow::bail!("keystore::load is not yet implemented")
}
