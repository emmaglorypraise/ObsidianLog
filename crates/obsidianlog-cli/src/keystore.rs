//! Local credential storage (ADR-0015): the archive's AES-256 encryption
//! key, and — when the Sia backend is configured — the indexd application
//! key, stored together as one [`CredentialBundle`].
//!
//! Generated/collected once during `obsidianlog init` and never
//! transmitted. Stored in the OS keychain (Keychain on macOS, Credential
//! Manager on Windows, Secret Service on Linux) via the `keyring` crate, or
//! — when the keychain is genuinely unreachable — a `0600` local file. This
//! module is the only place credentials are read or written.
//!
//! On macOS specifically, creating a fresh bundle uses a direct, "create
//! only" write (see [`BundleStore::create`]) rather than the generic
//! `keyring` crate's check-then-write pattern, so a fresh `obsidianlog
//! init` costs exactly one keychain authorization, not two. See ADR-0015
//! for the full design and the reasoning behind it.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Service name used to namespace ObsidianLog entries in the OS keychain.
pub const KEYRING_SERVICE: &str = "obsidianlog";
/// Keychain account / file name for the bundled credential.
pub const CREDENTIAL_BUNDLE_ACCOUNT: &str = "credentials";

/// The encryption key and, if the Sia backend is configured, the Sia app
/// key — stored together as one credential rather than as two independent
/// keychain items (ADR-0015).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialBundle {
    pub encryption_key: [u8; 32],
    pub sia_app_key: Option<[u8; 32]>,
}

impl CredentialBundle {
    fn to_json(&self) -> String {
        serde_json::to_string(self).expect("CredentialBundle always serializes")
    }

    fn from_json(text: &str) -> Result<Self> {
        serde_json::from_str(text).context("the stored credential bundle is not valid")
    }
}

/// The result of [`BundleStore::create`]: whether a new bundle was written,
/// or one was already there and preserved untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleCreateOutcome {
    Created,
    AlreadyExists,
}

/// Where the bundled credential lives. Implemented by [`KeyringBundleStore`]
/// (the OS keychain) and [`FileBundleStore`] (a `0600` file); tests use
/// [`MockBundleStore`] so they never touch the real keychain or filesystem.
pub trait BundleStore {
    /// Read the stored bundle, or `None` if nothing is stored yet.
    fn read(&self) -> Result<Option<CredentialBundle>>;

    /// Unconditionally persist `bundle`, overwriting any existing one. Used
    /// for explicit rotation and for a repair that merges in new credential
    /// material — both read first, then write, deliberately never trying to
    /// collapse that into one call (ADR-0015).
    fn write(&self, bundle: &CredentialBundle) -> Result<()>;

    /// Write `bundle` only if nothing is stored yet. Returns
    /// [`BundleCreateOutcome::AlreadyExists`], leaving the existing bundle
    /// untouched, if one is already present — never overwrites.
    fn create(&self, bundle: &CredentialBundle) -> Result<BundleCreateOutcome>;

    /// A short, human-readable description of where the bundle lives (e.g.
    /// "the OS keychain" or a file path), for prompts and confirmations.
    fn describe(&self) -> String;
}

/// Stores the bundle in the OS keychain via the `keyring` crate.
pub struct KeyringBundleStore {
    service: String,
    account: String,
}

impl KeyringBundleStore {
    pub fn new() -> Self {
        Self {
            service: KEYRING_SERVICE.to_string(),
            account: CREDENTIAL_BUNDLE_ACCOUNT.to_string(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, &self.account).context("opening the OS keychain entry")
    }
}

impl Default for KeyringBundleStore {
    fn default() -> Self {
        Self::new()
    }
}

impl BundleStore for KeyringBundleStore {
    fn read(&self) -> Result<Option<CredentialBundle>> {
        match self.entry()?.get_password() {
            Ok(text) => Ok(Some(CredentialBundle::from_json(&text)?)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e).context("reading the credential bundle from the OS keychain"),
        }
    }

    fn write(&self, bundle: &CredentialBundle) -> Result<()> {
        self.entry()?
            .set_password(&bundle.to_json())
            .context("writing the credential bundle to the OS keychain")
    }

    #[cfg(target_os = "macos")]
    fn create(&self, bundle: &CredentialBundle) -> Result<BundleCreateOutcome> {
        macos::create_only(&self.service, &self.account, bundle.to_json().as_bytes())
    }

    /// Windows and Linux have different credential-store write semantics
    /// and no evidence of the same multi-prompt problem macOS has, so they
    /// keep the generic check-then-write pattern here (ADR-0015) — still
    /// one item instead of two, just without a specific one-call claim.
    #[cfg(not(target_os = "macos"))]
    fn create(&self, bundle: &CredentialBundle) -> Result<BundleCreateOutcome> {
        if self.read()?.is_some() {
            return Ok(BundleCreateOutcome::AlreadyExists);
        }
        self.write(bundle)?;
        Ok(BundleCreateOutcome::Created)
    }

    fn describe(&self) -> String {
        "the OS keychain".to_string()
    }
}

/// The macOS-specific create-only path (ADR-0015): calls the lower-level
/// keychain API directly instead of the `keyring` crate's `set_password`,
/// which always does its own internal existence check before writing.
#[cfg(target_os = "macos")]
mod macos {
    use super::{BundleCreateOutcome, Result};
    use anyhow::Context;
    use security_framework::os::macos::keychain::SecKeychain;

    /// `errSecDuplicateItem` — the item already exists.
    const ERR_SEC_DUPLICATE_ITEM: i32 = -25299;

    pub(super) fn create_only(
        service: &str,
        account: &str,
        secret: &[u8],
    ) -> Result<BundleCreateOutcome> {
        let keychain = SecKeychain::default().context("opening the default macOS keychain")?;
        match keychain.add_generic_password(service, account, secret) {
            Ok(()) => Ok(BundleCreateOutcome::Created),
            Err(e) if e.code() == ERR_SEC_DUPLICATE_ITEM => Ok(BundleCreateOutcome::AlreadyExists),
            Err(e) => Err(anyhow::Error::from(e))
                .context("creating the credential bundle in the OS keychain"),
        }
    }
}

/// Stores the bundle as JSON in a file with `0600` permissions (best-effort
/// on platforms without POSIX permission bits) — the fallback when the OS
/// keychain is genuinely unreachable.
pub struct FileBundleStore {
    path: PathBuf,
}

impl FileBundleStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The default fallback location: `~/.obsidianlog/credentials.json`,
    /// matching this repo's `.gitignore` (`/.obsidianlog`).
    pub fn default_path() -> Result<PathBuf> {
        let home = std::env::var("HOME")
            .context("HOME is not set; cannot resolve the default secrets path")?;
        Ok(PathBuf::from(home)
            .join(".obsidianlog")
            .join("credentials.json"))
    }

    fn ensure_parent(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating secrets directory {}", parent.display()))?;
        }
        Ok(())
    }

    #[cfg(unix)]
    fn restrict_permissions(&self) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("setting permissions on {}", self.path.display()))
    }

    #[cfg(not(unix))]
    fn restrict_permissions(&self) -> Result<()> {
        Ok(())
    }
}

impl BundleStore for FileBundleStore {
    fn read(&self) -> Result<Option<CredentialBundle>> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(CredentialBundle::from_json(&text)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e)
                .with_context(|| format!("reading credential file at {}", self.path.display())),
        }
    }

    fn write(&self, bundle: &CredentialBundle) -> Result<()> {
        self.ensure_parent()?;
        std::fs::write(&self.path, bundle.to_json())
            .with_context(|| format!("writing credential file at {}", self.path.display()))?;
        self.restrict_permissions()
    }

    fn create(&self, bundle: &CredentialBundle) -> Result<BundleCreateOutcome> {
        self.ensure_parent()?;
        use std::io::Write;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)
        {
            Ok(mut file) => {
                file.write_all(bundle.to_json().as_bytes())
                    .with_context(|| {
                        format!("writing credential file at {}", self.path.display())
                    })?;
                drop(file);
                self.restrict_permissions()?;
                Ok(BundleCreateOutcome::Created)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                Ok(BundleCreateOutcome::AlreadyExists)
            }
            Err(e) => Err(e)
                .with_context(|| format!("creating credential file at {}", self.path.display())),
        }
    }

    fn describe(&self) -> String {
        self.path.display().to_string()
    }
}

/// Whether `err` (from a [`BundleStore`] call against [`KeyringBundleStore`])
/// indicates the OS keychain itself is genuinely unreachable, as opposed to
/// a user-driven cancellation/denial or any other failure that should
/// surface rather than silently trigger the file-store fallback.
fn is_keychain_unavailable(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<keyring::Error>(),
        Some(keyring::Error::NoStorageAccess(_))
    )
}

/// Run `op` against the OS keychain bundle store; if that fails
/// specifically because the keychain itself is unreachable, retry the same
/// operation against the file-backed fallback instead. Any other error —
/// notably the user cancelling or denying an authorization prompt —
/// propagates rather than silently redirecting to a file. Returns the
/// result alongside a description of wherever it ended up, for user-facing
/// messages.
fn with_bundle_fallback<T>(op: impl Fn(&dyn BundleStore) -> Result<T>) -> Result<(T, String)> {
    let keyring = KeyringBundleStore::new();
    match op(&keyring) {
        Ok(v) => Ok((v, keyring.describe())),
        Err(e) if is_keychain_unavailable(&e) => {
            let file = FileBundleStore::new(FileBundleStore::default_path()?);
            let v = op(&file)?;
            Ok((v, file.describe()))
        }
        Err(e) => Err(e),
    }
}

/// Read the stored credential bundle, if any, from the OS keychain (or the
/// file fallback). Returns `None` when nothing has been stored yet.
pub fn read_bundle() -> Result<(Option<CredentialBundle>, String)> {
    with_bundle_fallback(|store| store.read())
}

/// Load the stored credential bundle, erroring clearly if `obsidianlog
/// init` hasn't been run yet. Used by `serve`/`query`/backend resolution.
pub fn load_credential_bundle() -> Result<CredentialBundle> {
    let (bundle, _) = read_bundle()?;
    bundle.ok_or_else(|| anyhow::anyhow!("no credentials found — run `obsidianlog init` first"))
}

/// Create the bundle only if nothing is stored yet — the fresh-install and
/// pure-reuse-repair path (ADR-0015): one call, one keychain prompt.
pub fn create_bundle(bundle: &CredentialBundle) -> Result<(BundleCreateOutcome, String)> {
    with_bundle_fallback(|store| store.create(bundle))
}

/// Unconditionally overwrite the stored bundle — used for explicit
/// rotation and for a repair that merges in new credential material, both
/// of which read the existing bundle first (ADR-0015).
pub fn write_bundle(bundle: &CredentialBundle) -> Result<String> {
    with_bundle_fallback(|store| store.write(bundle)).map(|(_, desc)| desc)
}

/// The production [`BundleStore`]: adapts the free-function keychain-with-
/// file-fallback behavior above into a single implementation, so `init`'s
/// core logic can be written against one [`BundleStore`] regardless of
/// which concrete backend a given call ends up using. Tracks the
/// description of whichever backend last succeeded, for [`describe`].
///
/// [`describe`]: BundleStore::describe
pub struct DefaultBundleStore {
    last_location: std::cell::RefCell<String>,
}

impl DefaultBundleStore {
    pub fn new() -> Self {
        Self {
            last_location: std::cell::RefCell::new("the OS keychain".to_string()),
        }
    }
}

impl Default for DefaultBundleStore {
    fn default() -> Self {
        Self::new()
    }
}

impl BundleStore for DefaultBundleStore {
    fn read(&self) -> Result<Option<CredentialBundle>> {
        let (v, location) = with_bundle_fallback(|store| store.read())?;
        *self.last_location.borrow_mut() = location;
        Ok(v)
    }

    fn write(&self, bundle: &CredentialBundle) -> Result<()> {
        let (_, location) = with_bundle_fallback(|store| store.write(bundle))?;
        *self.last_location.borrow_mut() = location;
        Ok(())
    }

    fn create(&self, bundle: &CredentialBundle) -> Result<BundleCreateOutcome> {
        let (v, location) = with_bundle_fallback(|store| store.create(bundle))?;
        *self.last_location.borrow_mut() = location;
        Ok(v)
    }

    fn describe(&self) -> String {
        self.last_location.borrow().clone()
    }
}

/// An in-memory [`BundleStore`] for tests, so they never touch the real OS
/// keychain or filesystem. Also counts calls to each method, so tests can
/// assert exactly how many keychain round-trips a code path costs — not
/// just that the end state is correct.
#[cfg(test)]
pub struct MockBundleStore {
    bundle: std::sync::Mutex<Option<CredentialBundle>>,
    read_calls: std::sync::atomic::AtomicUsize,
    write_calls: std::sync::atomic::AtomicUsize,
    create_calls: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl MockBundleStore {
    pub fn empty() -> Self {
        Self {
            bundle: std::sync::Mutex::new(None),
            read_calls: std::sync::atomic::AtomicUsize::new(0),
            write_calls: std::sync::atomic::AtomicUsize::new(0),
            create_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn seeded(bundle: CredentialBundle) -> Self {
        Self {
            bundle: std::sync::Mutex::new(Some(bundle)),
            read_calls: std::sync::atomic::AtomicUsize::new(0),
            write_calls: std::sync::atomic::AtomicUsize::new(0),
            create_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn read_calls(&self) -> usize {
        self.read_calls.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn write_calls(&self) -> usize {
        self.write_calls.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn create_calls(&self) -> usize {
        self.create_calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
impl Default for MockBundleStore {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
impl BundleStore for MockBundleStore {
    fn read(&self) -> Result<Option<CredentialBundle>> {
        self.read_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self.bundle.lock().unwrap().clone())
    }

    fn write(&self, bundle: &CredentialBundle) -> Result<()> {
        self.write_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        *self.bundle.lock().unwrap() = Some(bundle.clone());
        Ok(())
    }

    fn create(&self, bundle: &CredentialBundle) -> Result<BundleCreateOutcome> {
        self.create_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut guard = self.bundle.lock().unwrap();
        if guard.is_some() {
            return Ok(BundleCreateOutcome::AlreadyExists);
        }
        *guard = Some(bundle.clone());
        Ok(BundleCreateOutcome::Created)
    }

    fn describe(&self) -> String {
        "a mock bundle store (tests only)".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(seed: u8) -> CredentialBundle {
        CredentialBundle {
            encryption_key: [seed; 32],
            sia_app_key: None,
        }
    }

    fn platform_error() -> Box<dyn std::error::Error + Send + Sync> {
        Box::new(std::io::Error::other("test platform error"))
    }

    #[test]
    fn no_storage_access_is_treated_as_keychain_unavailable() {
        let err = anyhow::Error::new(keyring::Error::NoStorageAccess(platform_error()));
        assert!(is_keychain_unavailable(&err));
    }

    #[test]
    fn platform_failure_is_not_treated_as_keychain_unavailable() {
        // Covers cancellation/denial, which the keyring crate also surfaces
        // as `PlatformFailure` — this must NOT silently fall back to the
        // file store, it must surface as a real error.
        let err = anyhow::Error::new(keyring::Error::PlatformFailure(platform_error()));
        assert!(!is_keychain_unavailable(&err));
    }

    #[test]
    fn no_entry_is_not_treated_as_keychain_unavailable() {
        let err = anyhow::Error::new(keyring::Error::NoEntry);
        assert!(!is_keychain_unavailable(&err));
    }

    #[test]
    fn an_unrelated_error_is_not_treated_as_keychain_unavailable() {
        let err = anyhow::anyhow!("some unrelated error");
        assert!(!is_keychain_unavailable(&err));
    }

    #[test]
    fn credential_bundle_round_trips_through_json() {
        let original = CredentialBundle {
            encryption_key: [0x42; 32],
            sia_app_key: Some([0x24; 32]),
        };
        let parsed = CredentialBundle::from_json(&original.to_json()).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn credential_bundle_without_a_sia_key_round_trips() {
        let original = bundle(0x11);
        let parsed = CredentialBundle::from_json(&original.to_json()).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn mock_bundle_store_create_then_read_round_trips() {
        let store = MockBundleStore::empty();
        assert!(store.read().unwrap().is_none());

        let outcome = store.create(&bundle(1)).unwrap();
        assert_eq!(outcome, BundleCreateOutcome::Created);
        assert_eq!(store.read().unwrap(), Some(bundle(1)));
    }

    #[test]
    fn mock_bundle_store_create_preserves_an_existing_bundle() {
        let store = MockBundleStore::seeded(bundle(1));
        let outcome = store.create(&bundle(2)).unwrap();
        assert_eq!(outcome, BundleCreateOutcome::AlreadyExists);
        assert_eq!(
            store.read().unwrap(),
            Some(bundle(1)),
            "create must never overwrite an existing bundle"
        );
    }

    #[test]
    fn mock_bundle_store_write_overwrites() {
        let store = MockBundleStore::seeded(bundle(1));
        store.write(&bundle(2)).unwrap();
        assert_eq!(store.read().unwrap(), Some(bundle(2)));
    }

    #[test]
    fn file_bundle_store_create_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileBundleStore::new(dir.path().join("credentials.json"));

        assert!(store.read().unwrap().is_none());
        let outcome = store.create(&bundle(7)).unwrap();
        assert_eq!(outcome, BundleCreateOutcome::Created);
        assert_eq!(store.read().unwrap(), Some(bundle(7)));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("credentials.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "credential file must be 0600");
        }
    }

    #[test]
    fn file_bundle_store_create_preserves_an_existing_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileBundleStore::new(dir.path().join("credentials.json"));
        store.write(&bundle(1)).unwrap();

        let outcome = store.create(&bundle(2)).unwrap();
        assert_eq!(outcome, BundleCreateOutcome::AlreadyExists);
        assert_eq!(store.read().unwrap(), Some(bundle(1)));
    }

    #[test]
    fn file_bundle_store_write_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileBundleStore::new(dir.path().join("credentials.json"));
        store.write(&bundle(1)).unwrap();
        store.write(&bundle(2)).unwrap();
        assert_eq!(store.read().unwrap(), Some(bundle(2)));
    }
}
