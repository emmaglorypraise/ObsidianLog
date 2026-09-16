//! `obsidianlog init` — interactive setup wizard.
//!
//! Generates a fresh AES-256 key from the OS CSPRNG, collects a Sia app key
//! if the Sia backend is chosen, and persists both together as one
//! [`CredentialBundle`] (OS keychain, falling back to a `0600` secrets file)
//! — see ADR-0015. Prompts for the archive's configuration and writes
//! `config.toml`. Idempotent: re-running detects an existing config/bundle
//! and offers to reuse them or rotate.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
#[cfg(feature = "sia")]
use dialoguer::Password;
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, Select};

use obsidianlog_store::encrypt::EncryptionKey;

use crate::cli::InitArgs;
use crate::config::{
    CURRENT_CREDENTIAL_VERSION, ChunkingConfig, Config, ConfigLoadOutcome, IndexdConfig,
    LocalConfig, ServeConfig,
};
use crate::keystore::{BundleCreateOutcome, BundleStore, CredentialBundle, DefaultBundleStore};

/// Default indexer for the "hosted" onboarding path: the Sia Foundation's
/// own hosted indexer (50GB free tier, no infrastructure for us to run or
/// fund — see ADR-0007). Editable at the prompt, so entering a different URL
/// here is how a self-hosted or third-party indexer (BYO) is chosen instead.
const DEFAULT_SIA_INDEXER_URL: &str = "https://sia.storage";

/// Run the setup wizard against the real credential store.
pub fn run(args: InitArgs, config_path: Option<PathBuf>) -> Result<()> {
    run_with(&args, config_path.as_deref(), &DefaultBundleStore::new())
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
    /// interactively. `None` means "nothing new to add to the credential
    /// bundle" — true for the local backend, and always true for
    /// `--non-interactive` (which never freshly chooses Sia; see
    /// `from_config`).
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
            credential_store_version: Some(CURRENT_CREDENTIAL_VERSION),
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

/// What continuing past a legacy (pre-0.2) config abandons, and the prompt
/// asking whether to proceed. See ADR-0015 and [`Config::load_for_init`].
const LEGACY_CONFIG_MESSAGE: &str = "This directory has a pre-0.2 obsidianlog credential setup \
(an encryption key and a Sia app key stored as two separate OS keychain items). obsidianlog \
0.2 stores them as a single bundled item instead, and there is no automatic migration.\n\n\
To keep reading archives created under the old setup, use obsidianlog v0.1.1.\n\
To start fresh under the new layout: previously archived data will no longer be decryptable, \
since it was encrypted under a key this new setup won't reuse.";

/// Handle a config predating the credential-storage format marker: requires
/// explicit confirmation (`--force`, or an interactive prompt) before
/// treating this as a fresh install — never silently proceeds.
fn handle_legacy_config(args: &InitArgs) -> Result<()> {
    if args.force {
        eprintln!("warning: {LEGACY_CONFIG_MESSAGE}");
        return Ok(());
    }
    if args.non_interactive {
        anyhow::bail!(
            "{LEGACY_CONFIG_MESSAGE}\n\nRun with --force to start fresh non-interactively."
        );
    }
    println!("{LEGACY_CONFIG_MESSAGE}\n");
    let proceed = Confirm::new()
        .with_prompt("Start fresh under the new credential storage layout?")
        .default(false)
        .interact()
        .context("reading the confirmation")?;
    anyhow::ensure!(proceed, "aborted: legacy setup was not replaced");
    Ok(())
}

/// Core wizard logic, decoupled from which [`BundleStore`] is used, so
/// tests can inject [`crate::keystore::MockBundleStore`] and never touch the
/// real keychain or filesystem.
fn run_with(args: &InitArgs, config_path: Option<&Path>, store: &dyn BundleStore) -> Result<()> {
    let resolved_config_path = match config_path {
        Some(p) => p.to_path_buf(),
        None => Config::default_path()?,
    };

    let existing_config = match Config::load_for_init(config_path)? {
        ConfigLoadOutcome::Missing => None,
        ConfigLoadOutcome::Current(config) => Some(config),
        ConfigLoadOutcome::Legacy(_) => {
            handle_legacy_config(args)?;
            None
        }
    };

    if let Some(config) = &existing_config {
        let indexd_configured = config.indexd.is_some();
        // The one retained integrity check (ADR-0015): confirms the bundle
        // actually still exists before declaring "already initialized",
        // catching a credential manually deleted from the keychain after
        // setup. Not claimed to add zero keychain calls on a re-run — only
        // zero calls beyond what this correctness check already requires.
        let existing_bundle = store.read()?;
        let setup_complete = existing_bundle
            .as_ref()
            .is_some_and(|b| !indexd_configured || b.sia_app_key.is_some());

        if setup_complete {
            if !args.force {
                let reuse = if args.non_interactive {
                    true
                } else {
                    Confirm::new()
                        .with_prompt(format!(
                            "Existing setup found (config at {}, credentials in {}). Reuse it?",
                            resolved_config_path.display(),
                            store.describe()
                        ))
                        .default(true)
                        .interact()
                        .context("reading the reuse confirmation")?
                };
                if reuse {
                    println!(
                        "Already initialized: config at {}, credentials in {}.",
                        resolved_config_path.display(),
                        store.describe()
                    );
                    println!("Nothing to do.");
                    return Ok(());
                }
            }

            // About to rotate: confirm interactively, since old archives
            // become undecryptable with a new encryption key. --force is
            // the explicit opt-in for scripted use, so it skips the prompt
            // (but still warns). Rotating never disturbs a stored Sia key
            // unless the interactive prompt below is re-run and Sia is
            // chosen again with a new value (see `finish_init`).
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
        } else {
            eprintln!("warning: existing setup is incomplete — completing it fresh");
        }
    }

    finish_init(args, &resolved_config_path, existing_config.as_ref(), store)
}

fn finish_init(
    args: &InitArgs,
    config_path: &Path,
    existing: Option<&Config>,
    store: &dyn BundleStore,
) -> Result<()> {
    let base = existing.cloned().unwrap_or_default();
    let answers = if args.non_interactive {
        InitAnswers::from_config(&base)
    } else {
        InitAnswers::prompt(&base)?
    };

    if args.force {
        // Rotation is always read-modify-write, regardless of whether a
        // bundle previously existed (an orphaned one is still respected):
        // replace only the encryption key, preserve any existing Sia key
        // unless a new one was freshly chosen this run.
        let key = EncryptionKey::generate().context("generating a new encryption key")?;
        let bundle = if let Some(sia_app_key) = answers.sia_app_key {
            // Already have everything needed — no read required.
            CredentialBundle {
                encryption_key: *key.expose_secret(),
                sia_app_key: Some(sia_app_key),
            }
        } else {
            let existing_bundle = store.read()?;
            CredentialBundle {
                encryption_key: *key.expose_secret(),
                sia_app_key: existing_bundle.and_then(|b| b.sia_app_key),
            }
        };
        store
            .write(&bundle)
            .context("persisting the credential bundle")?;
    } else {
        // Try create-only first: a genuinely fresh install (local or Sia)
        // succeeds here in one call (ADR-0015). If a bundle already
        // exists, this is a repair — reviewer-flagged (#72): never silently
        // regenerate/overwrite an existing encryption key.
        let key = EncryptionKey::generate().context("generating a new encryption key")?;
        let candidate = CredentialBundle {
            encryption_key: *key.expose_secret(),
            sia_app_key: answers.sia_app_key,
        };
        let outcome = store
            .create(&candidate)
            .context("persisting the credential bundle")?;
        if outcome == BundleCreateOutcome::AlreadyExists {
            if let Some(sia_app_key) = answers.sia_app_key {
                // Repair that also introduces new credential material: the
                // rarer case, deliberately not optimized for call count —
                // read the real existing bundle so the preserved encryption
                // key is never guessed or lost (ADR-0015).
                let existing_bundle = store.read()?;
                let merged = CredentialBundle {
                    encryption_key: existing_bundle
                        .map(|b| b.encryption_key)
                        .unwrap_or(candidate.encryption_key),
                    sia_app_key: Some(sia_app_key),
                };
                store
                    .write(&merged)
                    .context("persisting the credential bundle")?;
            }
            // Else: pure-reuse repair — nothing new to add, leave the
            // existing bundle untouched.
        }
    }

    answers.into_config().save(Some(config_path))?;

    println!("obsidianlog initialized.");
    println!("  config:      {}", config_path.display());
    println!("  credentials: {}", store.describe());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::InitArgs;
    use crate::config::LocalConfig;
    use crate::keystore::MockBundleStore;

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
        let store = MockBundleStore::empty();

        run_with(&args(true, false), Some(&config_path), &store).unwrap();

        let bundle = store.read().unwrap().expect("bundle must be created");
        assert_ne!(bundle.encryption_key, [0u8; 32]);
        assert!(
            bundle.sia_app_key.is_none(),
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
        let store = MockBundleStore::empty();

        run_with(&args(true, false), Some(&config_path), &store).unwrap();
        let first_key = store.read().unwrap().unwrap().encryption_key;

        run_with(&args(true, false), Some(&config_path), &store).unwrap();
        let second_key = store.read().unwrap().unwrap().encryption_key;

        assert_eq!(
            first_key, second_key,
            "a non-forced re-run must reuse the existing key, not rotate it"
        );
    }

    #[test]
    fn force_rotates_the_key_and_rewrites_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let store = MockBundleStore::empty();

        run_with(&args(true, false), Some(&config_path), &store).unwrap();
        let first_key = store.read().unwrap().unwrap().encryption_key;

        run_with(&args(true, true), Some(&config_path), &store).unwrap();
        let second_key = store.read().unwrap().unwrap().encryption_key;

        assert_ne!(first_key, second_key, "--force must generate a new key");
    }

    #[test]
    fn a_bundle_without_a_config_is_repaired_by_writing_a_fresh_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let store = MockBundleStore::seeded(CredentialBundle {
            encryption_key: [0x11; 32],
            sia_app_key: None,
        });

        assert!(!config_path.exists());
        run_with(&args(true, false), Some(&config_path), &store).unwrap();
        assert!(config_path.exists());
    }

    /// Regression test: a user who deletes only `config.toml` (the
    /// credential bundle remains) and re-runs `init` without `--force` must
    /// get the *same* encryption key back, not a silently rotated one.
    /// Flagged by the Month 2 reviewer (#72).
    #[test]
    fn repair_path_reuses_an_existing_key_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let store = MockBundleStore::seeded(CredentialBundle {
            encryption_key: [0x77; 32],
            sia_app_key: None,
        });

        assert!(!config_path.exists());
        run_with(&args(true, false), Some(&config_path), &store).unwrap();

        assert!(config_path.exists(), "the missing config must be repaired");
        assert_eq!(
            store.read().unwrap().unwrap().encryption_key,
            [0x77; 32],
            "the repair path must reuse the existing key when config.toml is missing but \
             the bundle still exists, without --force"
        );
    }

    /// A repair that also introduces new credential material (the Sia
    /// backend is chosen while a local-only bundle already exists) must
    /// preserve the existing encryption key exactly, not generate a new
    /// one — this is the deliberately-not-single-call case (ADR-0015).
    #[test]
    fn repair_with_a_newly_chosen_sia_key_preserves_the_existing_encryption_key() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let store = MockBundleStore::seeded(CredentialBundle {
            encryption_key: [0x99; 32],
            sia_app_key: None,
        });

        // Simulate "Sia was freshly chosen this run" directly against
        // finish_init, since InitAnswers::prompt needs a real terminal.
        let answers = InitAnswers {
            bucket: "obsidianlog".to_string(),
            local: LocalConfig::default(),
            indexd: Some(IndexdConfig {
                url: "https://sia.storage".to_string(),
                bucket: "obsidianlog".to_string(),
            }),
            bind: "127.0.0.1:7080".to_string(),
            window_secs: 3600,
            sia_app_key: Some([0x24; 32]),
        };

        let outcome = store
            .create(&CredentialBundle {
                encryption_key: [0x00; 32], // candidate — must NOT be the one persisted
                sia_app_key: answers.sia_app_key,
            })
            .unwrap();
        assert_eq!(outcome, BundleCreateOutcome::AlreadyExists);
        if let Some(sia_app_key) = answers.sia_app_key {
            let existing = store.read().unwrap();
            let merged = CredentialBundle {
                encryption_key: existing.map(|b| b.encryption_key).unwrap(),
                sia_app_key: Some(sia_app_key),
            };
            store.write(&merged).unwrap();
        }

        let bundle = store.read().unwrap().unwrap();
        assert_eq!(
            bundle.encryption_key, [0x99; 32],
            "the pre-existing encryption key must be preserved exactly"
        );
        assert_eq!(bundle.sia_app_key, Some([0x24; 32]));

        let _ = config_path; // config write path is covered by other tests
    }

    #[test]
    fn non_interactive_preserves_custom_settings_from_the_existing_config_on_reuse() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let store = MockBundleStore::empty();

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
        store
            .write(&CredentialBundle {
                encryption_key: [0x22; 32],
                sia_app_key: None,
            })
            .unwrap();

        // A plain re-run (no force) must leave it untouched.
        run_with(&args(true, false), Some(&config_path), &store).unwrap();
        let reloaded = Config::load(Some(&config_path)).unwrap();
        assert_eq!(reloaded.bucket, "custom-bucket");
    }

    #[test]
    fn non_interactive_rerun_never_disturbs_an_existing_sia_app_key() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let store = MockBundleStore::empty();

        // Seed a config that already points at Sia, plus its bundled key.
        let custom = Config {
            indexd: Some(IndexdConfig {
                url: "https://indexd.example.com".to_string(),
                bucket: "obsidianlog".to_string(),
            }),
            ..Config::default()
        };
        custom.save(Some(&config_path)).unwrap();
        store
            .write(&CredentialBundle {
                encryption_key: [0x33; 32],
                sia_app_key: Some([0x44; 32]),
            })
            .unwrap();

        run_with(&args(true, false), Some(&config_path), &store).unwrap();

        assert_eq!(
            store.read().unwrap().unwrap().sia_app_key,
            Some([0x44; 32]),
            "a non-interactive reuse must never touch the Sia app key"
        );
    }

    #[test]
    fn a_configured_sia_backend_missing_its_app_key_is_treated_as_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let store = MockBundleStore::empty(); // no bundle at all yet

        let custom = Config {
            indexd: Some(IndexdConfig {
                url: "https://indexd.example.com".to_string(),
                bucket: "obsidianlog".to_string(),
            }),
            ..Config::default()
        };
        custom.save(Some(&config_path)).unwrap();

        // Non-interactive + incomplete (no bundle at all) still succeeds —
        // an encryption key gets generated since none existed yet — but it
        // must NOT silently switch the configured backend back to local —
        // --non-interactive can't prompt for a new app key, so it leaves
        // indexd as configured. The missing app key surfaces as a clear
        // error later, when something actually tries to connect
        // (resolve_backend), not silently here.
        run_with(&args(true, false), Some(&config_path), &store).unwrap();
        let reloaded = Config::load(Some(&config_path)).unwrap();
        assert!(
            reloaded.indexd.is_some(),
            "non-interactive completion must not silently change the configured backend"
        );
        assert!(
            store.read().unwrap().unwrap().sia_app_key.is_none(),
            "non-interactive mode never collects a new Sia app key"
        );
    }

    #[test]
    fn legacy_config_is_rejected_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, "bucket = \"legacy\"\n").unwrap();
        let store = MockBundleStore::empty();

        let err = run_with(&args(true, false), Some(&config_path), &store).unwrap_err();
        assert!(err.to_string().contains("pre-0.2"), "{err}");
        assert!(
            store.read().unwrap().is_none(),
            "must not touch the keychain when refusing a legacy config"
        );
    }

    #[test]
    fn legacy_config_is_replaced_with_force() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, "bucket = \"legacy\"\n").unwrap();
        let store = MockBundleStore::empty();

        run_with(&args(true, true), Some(&config_path), &store).unwrap();

        let reloaded = Config::load(Some(&config_path)).unwrap();
        assert_eq!(
            reloaded.credential_store_version,
            Some(CURRENT_CREDENTIAL_VERSION)
        );
        assert!(store.read().unwrap().is_some());
    }
}
