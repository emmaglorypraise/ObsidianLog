//! User-controlled encryption key storage.
//!
//! Keys are generated locally during `obsidianlog init` and never transmitted.
//! They are stored in the OS keychain via the `keyring` crate (Linux/macOS/
//! Windows), or — when the keychain is unavailable — an explicit local secrets
//! file with `0600` permissions. This module is the only place keys are read
//! or written; [`EncryptionKey::expose_secret`] exists solely for this
//! boundary.
//!
//! Key storage is abstracted behind the [`KeyStore`] trait so callers (and
//! tests) aren't coupled to a specific backend. [`default_key_store`] is the
//! production resolution: try the OS keychain, fall back to a secrets file.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use obsidianlog_store::encrypt::{EncryptionKey, KEY_LEN};

/// Service name used to namespace ObsidianLog entries in the OS keychain.
pub const KEYRING_SERVICE: &str = "obsidianlog";
/// Account name under which the encryption key is stored (one key per
/// install; not per-bucket).
const KEY_ACCOUNT: &str = "encryption-key";

/// Where key storage backends persist and retrieve the archive's encryption
/// key. Implemented by [`KeyringStore`] (the OS keychain) and [`FileKeyStore`]
/// (a `0600` secrets file); tests use [`MockKeyStore`] so they never touch the
/// real keychain or filesystem.
pub trait KeyStore {
    /// Whether a key is already stored.
    fn exists(&self) -> Result<bool>;

    /// Durably persist `key`, overwriting any existing one.
    fn store(&self, key: &EncryptionKey) -> Result<()>;

    /// Load the previously stored key.
    fn load(&self) -> Result<EncryptionKey>;

    /// Remove the stored key (used when rotating).
    fn delete(&self) -> Result<()>;

    /// A short, human-readable description of where the key lives (e.g. "the
    /// OS keychain" or a file path), for prompts and confirmation messages.
    fn describe(&self) -> String;
}

/// Stores the key in the OS keychain (Keychain on macOS, Credential Manager on
/// Windows, Secret Service on Linux) via the `keyring` crate.
pub struct KeyringStore {
    service: String,
    account: String,
}

impl KeyringStore {
    /// The default keychain entry ObsidianLog uses.
    pub fn new() -> Self {
        Self {
            service: KEYRING_SERVICE.to_string(),
            account: KEY_ACCOUNT.to_string(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, &self.account).context("opening the OS keychain entry")
    }
}

impl Default for KeyringStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyStore for KeyringStore {
    fn exists(&self) -> Result<bool> {
        match self.entry()?.get_password() {
            Ok(_) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(e).context("checking the OS keychain"),
        }
    }

    fn store(&self, key: &EncryptionKey) -> Result<()> {
        let hex = to_hex(key.expose_secret());
        self.entry()?
            .set_password(&hex)
            .context("writing the key to the OS keychain")
    }

    fn load(&self) -> Result<EncryptionKey> {
        let hex = self
            .entry()?
            .get_password()
            .context("reading the key from the OS keychain")?;
        let bytes = from_hex(hex.trim()).context("OS keychain entry is not a valid key")?;
        Ok(EncryptionKey::new(bytes))
    }

    fn delete(&self) -> Result<()> {
        match self.entry()?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e).context("deleting the OS keychain entry"),
        }
    }

    fn describe(&self) -> String {
        "the OS keychain".to_string()
    }
}

/// Stores the key hex-encoded in a file with `0600` permissions (best-effort
/// on platforms without POSIX permission bits).
pub struct FileKeyStore {
    path: PathBuf,
}

impl FileKeyStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The default fallback location: `~/.obsidianlog/key.secret`, matching
    /// this repo's `.gitignore` (`/.obsidianlog`, `*.secret`).
    pub fn default_path() -> Result<PathBuf> {
        let home = std::env::var("HOME")
            .context("HOME is not set; cannot resolve the default key file path")?;
        Ok(PathBuf::from(home).join(".obsidianlog").join("key.secret"))
    }

    /// The path this store reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl KeyStore for FileKeyStore {
    fn exists(&self) -> Result<bool> {
        Ok(self.path.exists())
    }

    fn store(&self, key: &EncryptionKey) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating key directory {}", parent.display()))?;
        }
        let hex = to_hex(key.expose_secret());
        std::fs::write(&self.path, hex)
            .with_context(|| format!("writing key file at {}", self.path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("setting permissions on {}", self.path.display()))?;
        }
        Ok(())
    }

    fn load(&self) -> Result<EncryptionKey> {
        let text = std::fs::read_to_string(&self.path)
            .with_context(|| format!("reading key file at {}", self.path.display()))?;
        let bytes = from_hex(text.trim())
            .with_context(|| format!("key file at {} is not a valid key", self.path.display()))?;
        Ok(EncryptionKey::new(bytes))
    }

    fn delete(&self) -> Result<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => {
                Err(e).with_context(|| format!("deleting key file at {}", self.path.display()))
            }
        }
    }

    fn describe(&self) -> String {
        self.path.display().to_string()
    }
}

/// Resolve the [`KeyStore`] `obsidianlog` uses in production: the OS keychain
/// if it's reachable, otherwise a `0600` secrets file at
/// [`FileKeyStore::default_path`] (the caller should tell the user when this
/// fallback is taken).
pub fn default_key_store() -> Result<Box<dyn KeyStore>> {
    let keyring = KeyringStore::new();
    match keyring.exists() {
        Ok(_) => Ok(Box::new(keyring)),
        Err(_) => Ok(Box::new(FileKeyStore::new(FileKeyStore::default_path()?))),
    }
}

/// Encode `bytes` as lowercase hex.
fn to_hex(bytes: &[u8; KEY_LEN]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decode a lowercase (or uppercase) hex string into exactly [`KEY_LEN`] bytes.
fn from_hex(hex: &str) -> Result<[u8; KEY_LEN]> {
    anyhow::ensure!(
        hex.len() == KEY_LEN * 2,
        "expected {} hex characters, got {}",
        KEY_LEN * 2,
        hex.len()
    );
    let mut bytes = [0u8; KEY_LEN];
    for (i, byte) in bytes.iter_mut().enumerate() {
        let pair = &hex[i * 2..i * 2 + 2];
        *byte =
            u8::from_str_radix(pair, 16).with_context(|| format!("invalid hex byte {pair:?}"))?;
    }
    Ok(bytes)
}

/// An in-memory [`KeyStore`] for tests, so they never touch the real OS
/// keychain or filesystem.
#[cfg(test)]
pub struct MockKeyStore {
    key: std::sync::Mutex<Option<EncryptionKey>>,
}

#[cfg(test)]
impl MockKeyStore {
    pub fn new() -> Self {
        Self {
            key: std::sync::Mutex::new(None),
        }
    }

    pub fn empty() -> Self {
        Self::new()
    }

    pub fn seeded(key: EncryptionKey) -> Self {
        Self {
            key: std::sync::Mutex::new(Some(key)),
        }
    }
}

#[cfg(test)]
impl Default for MockKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl KeyStore for MockKeyStore {
    fn exists(&self) -> Result<bool> {
        Ok(self.key.lock().unwrap().is_some())
    }

    fn store(&self, key: &EncryptionKey) -> Result<()> {
        *self.key.lock().unwrap() = Some(key.clone());
        Ok(())
    }

    fn load(&self) -> Result<EncryptionKey> {
        self.key
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no key stored"))
    }

    fn delete(&self) -> Result<()> {
        *self.key.lock().unwrap() = None;
        Ok(())
    }

    fn describe(&self) -> String {
        "a mock key store (tests only)".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_every_byte_value() {
        let bytes: [u8; KEY_LEN] = std::array::from_fn(|i| i as u8);
        let hex = to_hex(&bytes);
        assert_eq!(hex.len(), KEY_LEN * 2);
        assert_eq!(from_hex(&hex).unwrap(), bytes);
    }

    #[test]
    fn from_hex_rejects_the_wrong_length() {
        assert!(from_hex("abcd").is_err());
    }

    #[test]
    fn from_hex_rejects_non_hex_characters() {
        let bad = "zz".repeat(KEY_LEN);
        assert!(from_hex(&bad).is_err());
    }

    #[test]
    fn mock_key_store_stores_and_retrieves() {
        let store = MockKeyStore::empty();
        assert!(!store.exists().unwrap());
        assert!(store.load().is_err());

        let key = EncryptionKey::generate().unwrap();
        store.store(&key).unwrap();
        assert!(store.exists().unwrap());
        assert_eq!(store.load().unwrap().expose_secret(), key.expose_secret());

        store.delete().unwrap();
        assert!(!store.exists().unwrap());
    }

    #[test]
    fn mock_key_store_seeded_starts_populated() {
        let key = EncryptionKey::generate().unwrap();
        let store = MockKeyStore::seeded(key.clone());
        assert!(store.exists().unwrap());
        assert_eq!(store.load().unwrap().expose_secret(), key.expose_secret());
    }

    #[test]
    fn file_key_store_round_trips_through_a_0600_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.secret");
        let store = FileKeyStore::new(&path);

        assert!(!store.exists().unwrap());
        let key = EncryptionKey::generate().unwrap();
        store.store(&key).unwrap();
        assert!(store.exists().unwrap());
        assert_eq!(store.load().unwrap().expose_secret(), key.expose_secret());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "key file must be 0600");
        }

        store.delete().unwrap();
        assert!(!store.exists().unwrap());
        // Deleting an already-absent key is not an error (used by rotation).
        store.delete().unwrap();
    }
}
