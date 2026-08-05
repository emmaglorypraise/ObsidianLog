//! User-controlled secret storage: the archive's encryption key, and — when
//! configured for Sia — the indexd application key.
//!
//! Secrets are generated/collected locally during `obsidianlog init` and never
//! transmitted. They are stored in the OS keychain via the `keyring` crate
//! (Linux/macOS/Windows), or — when the keychain is unavailable — an explicit
//! local secrets file with `0600` permissions. This module is the only place
//! secrets are read or written; [`EncryptionKey::expose_secret`] exists solely
//! for this boundary.
//!
//! [`KeyStore`] persists a raw 32-byte secret under a named account,
//! independent of what the bytes mean. [`default_encryption_key_store`] and
//! [`default_sia_app_key_store`] are the two named instances `obsidianlog`
//! uses — one archive can hold both an encryption key and (if archiving to
//! Sia) an indexd app key, each stored and rotated independently.
//!
//! [`EncryptionKey`]: obsidianlog_store::encrypt::EncryptionKey

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Length, in bytes, of every secret this module stores (both the AES-256 key
/// and the Sia `AppKey` happen to be 32 bytes).
const SECRET_LEN: usize = 32;

/// Service name used to namespace ObsidianLog entries in the OS keychain.
pub const KEYRING_SERVICE: &str = "obsidianlog";
/// Keychain account / secrets-file name for the archive's AES-256 encryption key.
pub const ENCRYPTION_KEY_ACCOUNT: &str = "encryption-key";
/// Keychain account / secrets-file name for the Sia indexd application key.
pub const SIA_APP_KEY_ACCOUNT: &str = "sia-app-key";

/// A 32-byte secret store, keyed by account name. Implemented by
/// [`KeyringStore`] (the OS keychain) and [`FileKeyStore`] (a `0600` secrets
/// file); tests use [`MockKeyStore`] so they never touch the real keychain or
/// filesystem.
pub trait KeyStore {
    /// Whether a secret is already stored.
    fn exists(&self) -> Result<bool>;

    /// Durably persist `secret`, overwriting any existing one.
    fn store(&self, secret: &[u8; SECRET_LEN]) -> Result<()>;

    /// Load the previously stored secret.
    fn load(&self) -> Result<[u8; SECRET_LEN]>;

    /// Remove the stored secret (used when rotating).
    fn delete(&self) -> Result<()>;

    /// A short, human-readable description of where the secret lives (e.g.
    /// "the OS keychain" or a file path), for prompts and confirmations.
    fn describe(&self) -> String;
}

/// Stores a secret in the OS keychain (Keychain on macOS, Credential Manager
/// on Windows, Secret Service on Linux) via the `keyring` crate.
pub struct KeyringStore {
    service: String,
    account: String,
}

impl KeyringStore {
    /// A keychain entry under the given `account` (e.g.
    /// [`ENCRYPTION_KEY_ACCOUNT`] or [`SIA_APP_KEY_ACCOUNT`]).
    pub fn new(account: impl Into<String>) -> Self {
        Self {
            service: KEYRING_SERVICE.to_string(),
            account: account.into(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, &self.account).context("opening the OS keychain entry")
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

    fn store(&self, secret: &[u8; SECRET_LEN]) -> Result<()> {
        let hex = to_hex(secret);
        self.entry()?
            .set_password(&hex)
            .context("writing the secret to the OS keychain")
    }

    fn load(&self) -> Result<[u8; SECRET_LEN]> {
        let hex = self
            .entry()?
            .get_password()
            .context("reading the secret from the OS keychain")?;
        from_hex(hex.trim()).context("OS keychain entry is not a valid secret")
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

/// Stores a secret hex-encoded in a file with `0600` permissions
/// (best-effort on platforms without POSIX permission bits).
pub struct FileKeyStore {
    path: PathBuf,
}

impl FileKeyStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The default fallback location for a secret file named `name` (e.g.
    /// `key.secret`, `sia-app-key.secret`): `~/.obsidianlog/<name>`, matching
    /// this repo's `.gitignore` (`/.obsidianlog`, `*.secret`).
    pub fn default_path(name: &str) -> Result<PathBuf> {
        let home = std::env::var("HOME")
            .context("HOME is not set; cannot resolve the default secrets path")?;
        Ok(PathBuf::from(home).join(".obsidianlog").join(name))
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

    fn store(&self, secret: &[u8; SECRET_LEN]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating secrets directory {}", parent.display()))?;
        }
        let hex = to_hex(secret);
        std::fs::write(&self.path, hex)
            .with_context(|| format!("writing secret file at {}", self.path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("setting permissions on {}", self.path.display()))?;
        }
        Ok(())
    }

    fn load(&self) -> Result<[u8; SECRET_LEN]> {
        let text = std::fs::read_to_string(&self.path)
            .with_context(|| format!("reading secret file at {}", self.path.display()))?;
        from_hex(text.trim())
            .with_context(|| format!("secret file at {} is not valid", self.path.display()))
    }

    fn delete(&self) -> Result<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => {
                Err(e).with_context(|| format!("deleting secret file at {}", self.path.display()))
            }
        }
    }

    fn describe(&self) -> String {
        self.path.display().to_string()
    }
}

/// Resolve a [`KeyStore`] for `account`/`file_name`: the OS keychain if
/// reachable, otherwise a `0600` secrets file at
/// `FileKeyStore::default_path(file_name)` (the caller should tell the user
/// when this fallback is taken).
fn default_key_store(account: &str, file_name: &str) -> Result<Box<dyn KeyStore>> {
    let keyring = KeyringStore::new(account);
    match keyring.exists() {
        Ok(_) => Ok(Box::new(keyring)),
        Err(_) => Ok(Box::new(FileKeyStore::new(FileKeyStore::default_path(
            file_name,
        )?))),
    }
}

/// The archive's AES-256 encryption key store — used by every command.
pub fn default_encryption_key_store() -> Result<Box<dyn KeyStore>> {
    default_key_store(ENCRYPTION_KEY_ACCOUNT, "key.secret")
}

/// The Sia indexd application key store — only consulted when `config.indexd`
/// is set (i.e. the Sia backend was chosen during `init`).
pub fn default_sia_app_key_store() -> Result<Box<dyn KeyStore>> {
    default_key_store(SIA_APP_KEY_ACCOUNT, "sia-app-key.secret")
}

/// Decode a hex string collected from the user (e.g. the `init` Sia AppKey
/// prompt) into a secret. Same format [`KeyStore`] persists internally.
pub(crate) fn decode_secret_hex(hex: &str) -> Result<[u8; SECRET_LEN]> {
    from_hex(hex)
}

/// Encode `bytes` as lowercase hex.
fn to_hex(bytes: &[u8; SECRET_LEN]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decode a lowercase (or uppercase) hex string into exactly [`SECRET_LEN`] bytes.
fn from_hex(hex: &str) -> Result<[u8; SECRET_LEN]> {
    anyhow::ensure!(
        hex.len() == SECRET_LEN * 2,
        "expected {} hex characters, got {}",
        SECRET_LEN * 2,
        hex.len()
    );
    let mut bytes = [0u8; SECRET_LEN];
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
    secret: std::sync::Mutex<Option<[u8; SECRET_LEN]>>,
}

#[cfg(test)]
impl MockKeyStore {
    pub fn new() -> Self {
        Self {
            secret: std::sync::Mutex::new(None),
        }
    }

    pub fn empty() -> Self {
        Self::new()
    }

    pub fn seeded(secret: [u8; SECRET_LEN]) -> Self {
        Self {
            secret: std::sync::Mutex::new(Some(secret)),
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
        Ok(self.secret.lock().unwrap().is_some())
    }

    fn store(&self, secret: &[u8; SECRET_LEN]) -> Result<()> {
        *self.secret.lock().unwrap() = Some(*secret);
        Ok(())
    }

    fn load(&self) -> Result<[u8; SECRET_LEN]> {
        self.secret
            .lock()
            .unwrap()
            .ok_or_else(|| anyhow::anyhow!("no secret stored"))
    }

    fn delete(&self) -> Result<()> {
        *self.secret.lock().unwrap() = None;
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
        let bytes: [u8; SECRET_LEN] = std::array::from_fn(|i| i as u8);
        let hex = to_hex(&bytes);
        assert_eq!(hex.len(), SECRET_LEN * 2);
        assert_eq!(from_hex(&hex).unwrap(), bytes);
    }

    #[test]
    fn from_hex_rejects_the_wrong_length() {
        assert!(from_hex("abcd").is_err());
    }

    #[test]
    fn from_hex_rejects_non_hex_characters() {
        let bad = "zz".repeat(SECRET_LEN);
        assert!(from_hex(&bad).is_err());
    }

    #[test]
    fn mock_key_store_stores_and_retrieves() {
        let store = MockKeyStore::empty();
        assert!(!store.exists().unwrap());
        assert!(store.load().is_err());

        let secret = [0x42u8; SECRET_LEN];
        store.store(&secret).unwrap();
        assert!(store.exists().unwrap());
        assert_eq!(store.load().unwrap(), secret);

        store.delete().unwrap();
        assert!(!store.exists().unwrap());
    }

    #[test]
    fn mock_key_store_seeded_starts_populated() {
        let secret = [0x24u8; SECRET_LEN];
        let store = MockKeyStore::seeded(secret);
        assert!(store.exists().unwrap());
        assert_eq!(store.load().unwrap(), secret);
    }

    #[test]
    fn file_key_store_round_trips_through_a_0600_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.secret");
        let store = FileKeyStore::new(&path);

        assert!(!store.exists().unwrap());
        let secret = [0xABu8; SECRET_LEN];
        store.store(&secret).unwrap();
        assert!(store.exists().unwrap());
        assert_eq!(store.load().unwrap(), secret);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "secret file must be 0600");
        }

        store.delete().unwrap();
        assert!(!store.exists().unwrap());
        // Deleting an already-absent secret is not an error (used by rotation).
        store.delete().unwrap();
    }

    #[test]
    fn two_named_stores_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let encryption = FileKeyStore::new(dir.path().join("key.secret"));
        let sia = FileKeyStore::new(dir.path().join("sia-app-key.secret"));

        encryption.store(&[0x11; SECRET_LEN]).unwrap();
        assert!(
            !sia.exists().unwrap(),
            "storing one must not affect the other"
        );

        sia.store(&[0x22; SECRET_LEN]).unwrap();
        assert_eq!(encryption.load().unwrap(), [0x11; SECRET_LEN]);
        assert_eq!(sia.load().unwrap(), [0x22; SECRET_LEN]);
    }
}
