//! `obsidianlog init` — interactive setup wizard.
//!
//! Generates a fresh AES-256 key from the OS CSPRNG, persists it via a
//! [`KeyStore`] (OS keychain, falling back to a `0600` secrets file), prompts
//! for the archive's configuration, and writes `config.toml`. If the Sia
//! backend is chosen, also collects and persists the indexd application key
//! via a second, independent [`KeyStore`]. Idempotent: re-running detects an
//! existing config/key(s) and offers to reuse them or rotate.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
#[cfg(feature = "sia")]
use dialoguer::Password;
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, Select};

use obsidianlog_store::encrypt::EncryptionKey;

use crate::cli::InitArgs;
use crate::config::{ChunkingConfig, Config, IndexdConfig, LocalConfig, ServeConfig};
use crate::keystore::{self, KeyStore};

/// Default indexer for the "hosted" onboarding path: the Sia Foundation's
/// own hosted indexer (50GB free tier, no infrastructure for us to run or
/// fund — see ADR-0007). Editable at the prompt, so entering a different URL
/// here is how a self-hosted or third-party indexer (BYO) is chosen instead.
const DEFAULT_SIA_INDEXER_URL: &str = "https://sia.storage";

/// Run the setup wizard: resolves the real key stores, then delegates to the
/// testable core.
pub fn run(args: InitArgs, config_path: Option<PathBuf>) -> Result<()> {
    let (encryption_key_store, encryption_key_exists) = keystore::default_encryption_key_store()?;
    let (sia_app_key_store, sia_app_key_exists) = keystore::default_sia_app_key_store()?;
    run_with(
        &args,
        config_path.as_deref(),
        encryption_key_store.as_ref(),
        sia_app_key_store.as_ref(),
        encryption_key_exists,
        sia_app_key_exists,
    )
}

/// The answers driving a `config.toml`, gathered either interactively or
/// (`--non-interactive`) from an existing/default config. Decoupled from both
/// CLI-arg parsing and terminal I/O, so it's directly unit-testable.
struct InitAnswers {
    bucket: String,
    local: LocalConfig,
    indexd: Option<IndexdConfig>,
    bind: String,
    window_secs: u64,
    /// The Sia AppKey collected *this run*, if the Sia backend was chosen
    /// interactively. `None` means "don't touch the Sia app-key store" — true
    /// for the local backend, and always true for `--non-interactive` (which
    /// never freshly chooses Sia; see `from_config`).
    sia_app_key: Option<[u8; 32]>,
}

impl InitAnswers {
    fn from_config(config: &Config) -> Self {
        Self {
            bucket: config.bucket.clone(),
            local: config.local.clone(),
            indexd: config.indexd.clone(),
            bind: config.serve.bind.clone(),
            window_secs: config.chunking.window_secs,
            sia_app_key: None,
        }
    }

    /// Prompt interactively, using `base` (the existing config, or
    /// [`Config::default`] on first run) to pre-fill each default.
    fn prompt(base: &Config) -> Result<Self> {
        let theme = ColorfulTheme::default();

        let bucket: String = Input::with_theme(&theme)
            .with_prompt("Storage bucket / namespace")
            .default(base.bucket.clone())
            .interact_text()
            .context("reading the bucket name")?;

        let backends = [
            "local (no Sia node needed)",
            "sia (archive to the Sia network)",
        ];
        let backend_idx = Select::with_theme(&theme)
            .with_prompt("Storage backend")
            .items(&backends)
            .default(if base.indexd.is_some() { 1 } else { 0 })
            .interact()
            .context("reading the storage backend choice")?;

        let (local, indexd, sia_app_key) = if backend_idx == 0 {
            let data_dir: String = Input::with_theme(&theme)
                .with_prompt("Local storage directory")
                .default(base.local.data_dir.display().to_string())
                .interact_text()
                .context("reading the storage directory")?;
            (
                LocalConfig {
                    data_dir: PathBuf::from(data_dir),
                },
                None,
                None,
            )
        } else {
            let url: String = Input::with_theme(&theme)
                .with_prompt(
                    "indexd application API URL (the default is Sia Storage's hosted \
                     indexer — enter a different URL to bring your own indexer instead)",
                )
                .default(sia_url_default(base))
                .interact_text()
                .context("reading the indexd URL")?;

            let sia_app_key = onboard_sia(&theme, &url)?;

            (
                base.local.clone(),
                Some(IndexdConfig {
                    url,
                    bucket: bucket.clone(),
                }),
                Some(sia_app_key),
            )
        };

        let bind: String = Input::with_theme(&theme)
            .with_prompt("Ingest server bind address")
            .default(base.serve.bind.clone())
            .interact_text()
            .context("reading the bind address")?;

        let window_secs: u64 = Input::with_theme(&theme)
            .with_prompt("Chunk time window (seconds)")
            .default(base.chunking.window_secs)
            .interact_text()
            .context("reading the chunk window")?;

        Ok(Self {
            bucket,
            local,
            indexd,
            bind,
            window_secs,
            sia_app_key,
        })
    }

    fn into_config(self) -> Config {
        Config {
            bucket: self.bucket,
            local: self.local,
            indexd: self.indexd,
            serve: ServeConfig { bind: self.bind },
            chunking: ChunkingConfig {
                window_secs: self.window_secs,
            },
        }
    }
}

/// The indexd URL to pre-fill the prompt with: the previously configured
/// one on re-run, otherwise [`DEFAULT_SIA_INDEXER_URL`].
fn sia_url_default(base: &Config) -> String {
    base.indexd
        .as_ref()
        .map(|i| i.url.clone())
        .unwrap_or_else(|| DEFAULT_SIA_INDEXER_URL.to_string())
}

/// Run the interactive Sia onboarding flow against `url`: prompt for a
/// recovery phrase (or generate one), request approval, and register —
/// blocking until the user approves. Returns the derived `AppKey`.
///
/// Without the `sia` feature, this build can't reach `indexd` at all; fails
/// clearly instead of prompting for a phrase that will never be usable.
#[cfg(feature = "sia")]
fn onboard_sia(theme: &ColorfulTheme, url: &str) -> Result<[u8; 32]> {
    let recovery_phrase: String = Password::with_theme(theme)
        .with_prompt("Sia recovery phrase (type `seed` to generate a new one)")
        .interact()
        .context("reading the recovery phrase")?;

    let recovery_phrase = if recovery_phrase.trim() == "seed" {
        let generated = obsidianlog_store::backend::sia::generate_recovery_phrase();
        println!(
            "\nYour new recovery phrase — this is your master key, save it securely, it \
             is never stored by obsidianlog:\n\n    {generated}\n"
        );
        generated
    } else {
        recovery_phrase
    };

    println!("\nConnecting to {url}...");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building the async runtime for Sia onboarding")?;
    runtime
        .block_on(obsidianlog_store::backend::sia::onboard(
            url,
            &recovery_phrase,
            |approval_url| {
                println!("\nOpen this URL and approve the app in your Sia account:\n");
                println!("    {approval_url}\n");
                println!("Waiting for approval (this blocks until you approve)...");
            },
        ))
        .context("onboarding with the Sia indexer")
}

#[cfg(not(feature = "sia"))]
fn onboard_sia(_theme: &ColorfulTheme, _url: &str) -> Result<[u8; 32]> {
    anyhow::bail!(
        "this build of obsidianlog was compiled without Sia support; rebuild with \
         `cargo build --features sia` to onboard a Sia indexer"
    )
}

/// Core wizard logic, decoupled from which [`KeyStore`]s are used, so tests
/// can inject [`crate::keystore::MockKeyStore`] and never touch the real
/// keychain or filesystem outside a temp dir.
fn run_with(
    args: &InitArgs,
    config_path: Option<&Path>,
    encryption_key_store: &dyn KeyStore,
    sia_app_key_store: &dyn KeyStore,
    encryption_key_exists: bool,
    sia_app_key_exists: bool,
) -> Result<()> {
    let resolved_config_path = match config_path {
        Some(p) => p.to_path_buf(),
        None => Config::default_path()?,
    };

    let existing_config = if resolved_config_path.exists() {
        Some(Config::load(Some(&resolved_config_path))?)
    } else {
        None
    };
    let indexd_configured = existing_config.as_ref().is_some_and(|c| c.indexd.is_some());
    let setup_complete = existing_config.is_some()
        && encryption_key_exists
        && (!indexd_configured || sia_app_key_exists);

    if setup_complete {
        if !args.force {
            let reuse = if args.non_interactive {
                true
            } else {
                Confirm::new()
                    .with_prompt(format!(
                        "Existing setup found (config at {}, key in {}). Reuse it?",
                        resolved_config_path.display(),
                        encryption_key_store.describe()
                    ))
                    .default(true)
                    .interact()
                    .context("reading the reuse confirmation")?
            };
            if reuse {
                println!(
                    "Already initialized: config at {}, key in {}.",
                    resolved_config_path.display(),
                    encryption_key_store.describe()
                );
                if indexd_configured {
                    println!("  Sia app key in {}.", sia_app_key_store.describe());
                }
                println!("Nothing to do.");
                return Ok(());
            }
        }

        // About to rotate: confirm interactively, since old archives become
        // undecryptable with a new encryption key. --force is the explicit
        // opt-in for scripted use, so it skips the prompt (but still warns).
        // Rotating the encryption key never touches the Sia app key (an
        // independent credential) unless the interactive prompt below is
        // re-run and Sia is chosen again with a new value.
        if args.force {
            eprintln!(
                "warning: rotating the encryption key — previously archived data will no \
                 longer be decryptable with the new key"
            );
        } else if !args.non_interactive {
            let proceed = Confirm::new()
                .with_prompt(
                    "Rotating the key means previously archived data can no longer be \
                     decrypted with the new key. Continue?",
                )
                .default(false)
                .interact()
                .context("reading the rotation confirmation")?;
            anyhow::ensure!(proceed, "aborted: key rotation was not confirmed");
        }
    } else if existing_config.is_some() || encryption_key_exists || sia_app_key_exists {
        eprintln!("warning: existing setup is incomplete or inconsistent — completing it fresh");
    }

    finish_init(
        args,
        &resolved_config_path,
        encryption_key_store,
        sia_app_key_store,
        existing_config.as_ref(),
    )
}

fn finish_init(
    args: &InitArgs,
    config_path: &Path,
    encryption_key_store: &dyn KeyStore,
    sia_app_key_store: &dyn KeyStore,
    existing: Option<&Config>,
) -> Result<()> {
    let key = EncryptionKey::generate().context("generating a new encryption key")?;
    encryption_key_store
        .store(key.expose_secret())
        .context("persisting the encryption key")?;

    let base = existing.cloned().unwrap_or_default();
    let answers = if args.non_interactive {
        InitAnswers::from_config(&base)
    } else {
        InitAnswers::prompt(&base)?
    };

    if let Some(sia_app_key) = &answers.sia_app_key {
        sia_app_key_store
            .store(sia_app_key)
            .context("persisting the Sia app key")?;
    }
    let stored_sia_app_key = answers.sia_app_key.is_some();

    answers.into_config().save(Some(config_path))?;

    println!("obsidianlog initialized.");
    println!("  config: {}", config_path.display());
    println!("  key:    {}", encryption_key_store.describe());
    if stored_sia_app_key {
        println!("  sia app key: {}", sia_app_key_store.describe());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::InitArgs;
    use crate::config::LocalConfig;
    use crate::keystore::MockKeyStore;

    fn args(non_interactive: bool, force: bool) -> InitArgs {
        InitArgs {
            non_interactive,
            force,
        }
    }

    #[test]
    fn sia_url_defaults_to_the_hosted_indexer_when_unconfigured() {
        assert_eq!(sia_url_default(&Config::default()), DEFAULT_SIA_INDEXER_URL);
    }

    #[test]
    fn sia_url_default_reuses_a_previously_configured_indexer() {
        let config = Config {
            indexd: Some(IndexdConfig {
                url: "https://my-own-indexer.example.com".to_string(),
                bucket: "obsidianlog".to_string(),
            }),
            ..Config::default()
        };
        assert_eq!(
            sia_url_default(&config),
            "https://my-own-indexer.example.com"
        );
    }

    #[test]
    fn fresh_init_generates_a_key_and_writes_a_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let encryption_key_store = MockKeyStore::empty();
        let sia_app_key_store = MockKeyStore::empty();

        let encryption_key_exists = encryption_key_store.exists().unwrap();
        let sia_app_key_exists = sia_app_key_store.exists().unwrap();
        run_with(
            &args(true, false),
            Some(&config_path),
            &encryption_key_store,
            &sia_app_key_store,
            encryption_key_exists,
            sia_app_key_exists,
        )
        .unwrap();

        assert!(encryption_key_store.exists().unwrap());
        let key = encryption_key_store.load().unwrap();
        assert_ne!(key, [0u8; 32]);
        assert!(
            !sia_app_key_store.exists().unwrap(),
            "local backend by default: no Sia app key"
        );

        let config = Config::load(Some(&config_path)).unwrap();
        assert_eq!(config.bucket, Config::default().bucket);
        assert!(config.indexd.is_none());
    }

    #[test]
    fn rerunning_non_interactively_is_idempotent_and_keeps_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let encryption_key_store = MockKeyStore::empty();
        let sia_app_key_store = MockKeyStore::empty();

        let encryption_key_exists = encryption_key_store.exists().unwrap();
        let sia_app_key_exists = sia_app_key_store.exists().unwrap();
        run_with(
            &args(true, false),
            Some(&config_path),
            &encryption_key_store,
            &sia_app_key_store,
            encryption_key_exists,
            sia_app_key_exists,
        )
        .unwrap();
        let first_key = encryption_key_store.load().unwrap();

        let encryption_key_exists = encryption_key_store.exists().unwrap();
        let sia_app_key_exists = sia_app_key_store.exists().unwrap();
        run_with(
            &args(true, false),
            Some(&config_path),
            &encryption_key_store,
            &sia_app_key_store,
            encryption_key_exists,
            sia_app_key_exists,
        )
        .unwrap();
        let second_key = encryption_key_store.load().unwrap();

        assert_eq!(
            first_key, second_key,
            "a non-forced re-run must reuse the existing key, not rotate it"
        );
    }

    #[test]
    fn force_rotates_the_key_and_rewrites_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let encryption_key_store = MockKeyStore::empty();
        let sia_app_key_store = MockKeyStore::empty();

        let encryption_key_exists = encryption_key_store.exists().unwrap();
        let sia_app_key_exists = sia_app_key_store.exists().unwrap();
        run_with(
            &args(true, false),
            Some(&config_path),
            &encryption_key_store,
            &sia_app_key_store,
            encryption_key_exists,
            sia_app_key_exists,
        )
        .unwrap();
        let first_key = encryption_key_store.load().unwrap();

        let encryption_key_exists = encryption_key_store.exists().unwrap();
        let sia_app_key_exists = sia_app_key_store.exists().unwrap();
        run_with(
            &args(true, true),
            Some(&config_path),
            &encryption_key_store,
            &sia_app_key_store,
            encryption_key_exists,
            sia_app_key_exists,
        )
        .unwrap();
        let second_key = encryption_key_store.load().unwrap();

        assert_ne!(first_key, second_key, "--force must generate a new key");
    }

    #[test]
    fn a_key_without_a_config_is_repaired_by_writing_a_fresh_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let encryption_key_store = MockKeyStore::seeded([0x11; 32]);
        let sia_app_key_store = MockKeyStore::empty();

        assert!(!config_path.exists());
        let encryption_key_exists = encryption_key_store.exists().unwrap();
        let sia_app_key_exists = sia_app_key_store.exists().unwrap();
        run_with(
            &args(true, false),
            Some(&config_path),
            &encryption_key_store,
            &sia_app_key_store,
            encryption_key_exists,
            sia_app_key_exists,
        )
        .unwrap();
        assert!(config_path.exists());
    }

    #[test]
    fn non_interactive_preserves_custom_settings_from_the_existing_config_on_reuse() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let encryption_key_store = MockKeyStore::empty();
        let sia_app_key_store = MockKeyStore::empty();

        // Seed a config with a non-default bucket, as if a prior interactive
        // run had customized it.
        let custom = Config {
            bucket: "custom-bucket".to_string(),
            local: LocalConfig {
                data_dir: dir.path().join("archive"),
            },
            ..Config::default()
        };
        custom.save(Some(&config_path)).unwrap();
        encryption_key_store.store(&[0x22; 32]).unwrap();

        // A plain re-run (no force) must leave it untouched.
        let encryption_key_exists = encryption_key_store.exists().unwrap();
        let sia_app_key_exists = sia_app_key_store.exists().unwrap();
        run_with(
            &args(true, false),
            Some(&config_path),
            &encryption_key_store,
            &sia_app_key_store,
            encryption_key_exists,
            sia_app_key_exists,
        )
        .unwrap();
        let reloaded = Config::load(Some(&config_path)).unwrap();
        assert_eq!(reloaded.bucket, "custom-bucket");
    }

    #[test]
    fn non_interactive_rerun_never_disturbs_an_existing_sia_app_key() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let encryption_key_store = MockKeyStore::empty();
        let sia_app_key_store = MockKeyStore::empty();

        // Seed a config that already points at Sia, plus its app key.
        let custom = Config {
            indexd: Some(IndexdConfig {
                url: "https://indexd.example.com".to_string(),
                bucket: "obsidianlog".to_string(),
            }),
            ..Config::default()
        };
        custom.save(Some(&config_path)).unwrap();
        encryption_key_store.store(&[0x33; 32]).unwrap();
        sia_app_key_store.store(&[0x44; 32]).unwrap();

        let encryption_key_exists = encryption_key_store.exists().unwrap();
        let sia_app_key_exists = sia_app_key_store.exists().unwrap();
        run_with(
            &args(true, false),
            Some(&config_path),
            &encryption_key_store,
            &sia_app_key_store,
            encryption_key_exists,
            sia_app_key_exists,
        )
        .unwrap();

        assert_eq!(
            sia_app_key_store.load().unwrap(),
            [0x44; 32],
            "a non-interactive reuse must never touch the Sia app key"
        );
    }

    #[test]
    fn a_configured_sia_backend_missing_its_app_key_is_treated_as_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let encryption_key_store = MockKeyStore::empty();
        let sia_app_key_store = MockKeyStore::empty(); // app key missing on purpose

        let custom = Config {
            indexd: Some(IndexdConfig {
                url: "https://indexd.example.com".to_string(),
                bucket: "obsidianlog".to_string(),
            }),
            ..Config::default()
        };
        custom.save(Some(&config_path)).unwrap();
        encryption_key_store.store(&[0x55; 32]).unwrap();

        // Non-interactive + incomplete (missing Sia app key) still succeeds,
        // but it must NOT silently switch the configured backend back to
        // local — --non-interactive can't prompt for a new app key, so it
        // leaves indexd as configured. The missing app key surfaces as a
        // clear error later, when something actually tries to connect
        // (resolve_backend), not silently here.
        let encryption_key_exists = encryption_key_store.exists().unwrap();
        let sia_app_key_exists = sia_app_key_store.exists().unwrap();
        run_with(
            &args(true, false),
            Some(&config_path),
            &encryption_key_store,
            &sia_app_key_store,
            encryption_key_exists,
            sia_app_key_exists,
        )
        .unwrap();
        let reloaded = Config::load(Some(&config_path)).unwrap();
        assert!(
            reloaded.indexd.is_some(),
            "non-interactive completion must not silently change the configured backend"
        );
        assert!(
            !sia_app_key_store.exists().unwrap(),
            "non-interactive mode never collects a new Sia app key"
        );
    }
}
