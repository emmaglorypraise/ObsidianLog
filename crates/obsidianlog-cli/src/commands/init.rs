//! `obsidianlog init` — interactive setup wizard.
//!
//! Generates a fresh AES-256 key from the OS CSPRNG, persists it via a
//! [`KeyStore`] (OS keychain, falling back to a `0600` secrets file), prompts
//! for the archive's configuration, and writes `config.toml`. Idempotent:
//! re-running detects an existing config/key and offers to reuse it or
//! rotate.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dialoguer::Password;
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, Select};

use obsidianlog_store::encrypt::EncryptionKey;

use crate::cli::InitArgs;
use crate::config::{ChunkingConfig, Config, IndexdConfig, LocalConfig, ServeConfig};
use crate::keystore::{self, KeyStore};

/// Run the setup wizard: resolves the real key store, then delegates to the
/// testable core.
pub fn run(args: InitArgs, config_path: Option<PathBuf>) -> Result<()> {
    let key_store = keystore::default_key_store()?;
    run_with(&args, config_path.as_deref(), key_store.as_ref())
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
}

impl InitAnswers {
    fn from_config(config: &Config) -> Self {
        Self {
            bucket: config.bucket.clone(),
            local: config.local.clone(),
            indexd: config.indexd.clone(),
            bind: config.serve.bind.clone(),
            window_secs: config.chunking.window_secs,
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

        let (local, indexd) = if backend_idx == 0 {
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
            )
        } else {
            let url: String = Input::with_theme(&theme)
                .with_prompt("indexd application API URL")
                .default(
                    base.indexd
                        .as_ref()
                        .map(|i| i.url.clone())
                        .unwrap_or_default(),
                )
                .interact_text()
                .context("reading the indexd URL")?;
            // Not persisted anywhere: the CLI doesn't wire indexd/Sia archival
            // yet (`serve` always uses the local backend), so there's no
            // consumer for this credential and no principled place to store
            // it securely for a feature that doesn't exist yet.
            let _app_password = Password::with_theme(&theme)
                .with_prompt(
                    "indexd application password (not stored — Sia-backend wiring is a future task)",
                )
                .allow_empty_password(true)
                .interact()
                .context("reading the indexd app password")?;
            (
                base.local.clone(),
                Some(IndexdConfig {
                    url,
                    bucket: bucket.clone(),
                }),
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

/// Core wizard logic, decoupled from which [`KeyStore`] is used, so tests can
/// inject [`crate::keystore::MockKeyStore`] and never touch the real
/// keychain or filesystem outside a temp dir.
fn run_with(args: &InitArgs, config_path: Option<&Path>, key_store: &dyn KeyStore) -> Result<()> {
    let resolved_config_path = match config_path {
        Some(p) => p.to_path_buf(),
        None => Config::default_path()?,
    };

    let existing_config = if resolved_config_path.exists() {
        Some(Config::load(Some(&resolved_config_path))?)
    } else {
        None
    };
    let key_exists = key_store.exists()?;

    if existing_config.is_some() {
        if key_exists && !args.force {
            let reuse = if args.non_interactive {
                true
            } else {
                Confirm::new()
                    .with_prompt(format!(
                        "Existing setup found (config at {}, key in {}). Reuse it?",
                        resolved_config_path.display(),
                        key_store.describe()
                    ))
                    .default(true)
                    .interact()
                    .context("reading the reuse confirmation")?
            };
            if reuse {
                println!(
                    "Already initialized: config at {}, key in {}. Nothing to do.",
                    resolved_config_path.display(),
                    key_store.describe()
                );
                return Ok(());
            }
        }

        if key_exists {
            // About to rotate: confirm interactively, since old archives
            // become undecryptable with the new key. --force is the explicit
            // opt-in for scripted use, so it skips the prompt (but still
            // warns).
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
        }
    } else if key_exists {
        eprintln!(
            "warning: found a stored key but no config at {} — completing setup fresh",
            resolved_config_path.display()
        );
    }

    finish_init(
        args,
        &resolved_config_path,
        key_store,
        existing_config.as_ref(),
    )
}

fn finish_init(
    args: &InitArgs,
    config_path: &Path,
    key_store: &dyn KeyStore,
    existing: Option<&Config>,
) -> Result<()> {
    key_store.delete()?; // no-op if nothing was stored yet
    let key = EncryptionKey::generate().context("generating a new encryption key")?;
    key_store
        .store(&key)
        .context("persisting the encryption key")?;

    let base = existing.cloned().unwrap_or_default();
    let answers = if args.non_interactive {
        InitAnswers::from_config(&base)
    } else {
        InitAnswers::prompt(&base)?
    };
    answers.into_config().save(Some(config_path))?;

    println!("obsidianlog initialized.");
    println!("  config: {}", config_path.display());
    println!("  key:    {}", key_store.describe());
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
    fn fresh_init_generates_a_key_and_writes_a_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let key_store = MockKeyStore::empty();

        run_with(&args(true, false), Some(&config_path), &key_store).unwrap();

        assert!(key_store.exists().unwrap());
        let key = key_store.load().unwrap();
        assert!(!key.is_placeholder());

        let config = Config::load(Some(&config_path)).unwrap();
        assert_eq!(config.bucket, Config::default().bucket);
    }

    #[test]
    fn rerunning_non_interactively_is_idempotent_and_keeps_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let key_store = MockKeyStore::empty();

        run_with(&args(true, false), Some(&config_path), &key_store).unwrap();
        let first_key = key_store.load().unwrap();

        run_with(&args(true, false), Some(&config_path), &key_store).unwrap();
        let second_key = key_store.load().unwrap();

        assert_eq!(
            first_key.expose_secret(),
            second_key.expose_secret(),
            "a non-forced re-run must reuse the existing key, not rotate it"
        );
    }

    #[test]
    fn force_rotates_the_key_and_rewrites_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let key_store = MockKeyStore::empty();

        run_with(&args(true, false), Some(&config_path), &key_store).unwrap();
        let first_key = key_store.load().unwrap();

        run_with(&args(true, true), Some(&config_path), &key_store).unwrap();
        let second_key = key_store.load().unwrap();

        assert_ne!(
            first_key.expose_secret(),
            second_key.expose_secret(),
            "--force must generate a new key"
        );
    }

    #[test]
    fn a_key_without_a_config_is_repaired_by_writing_a_fresh_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let key_store = MockKeyStore::seeded(EncryptionKey::generate().unwrap());

        assert!(!config_path.exists());
        run_with(&args(true, false), Some(&config_path), &key_store).unwrap();
        assert!(config_path.exists());
    }

    #[test]
    fn non_interactive_preserves_custom_settings_from_the_existing_config_on_reuse() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let key_store = MockKeyStore::empty();

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
        key_store
            .store(&EncryptionKey::generate().unwrap())
            .unwrap();

        // A plain re-run (no force) must leave it untouched.
        run_with(&args(true, false), Some(&config_path), &key_store).unwrap();
        let reloaded = Config::load(Some(&config_path)).unwrap();
        assert_eq!(reloaded.bucket, "custom-bucket");
    }
}
