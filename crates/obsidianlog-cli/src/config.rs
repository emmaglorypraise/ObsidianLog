//! Configuration model and loading.
//!
//! ObsidianLog is self-hosted: configuration captures where logs are archived
//! and how chunks are sized. It is **local-first** — the default backend writes
//! to a local directory and needs no Sia node (mock-first invariant). The
//! `indexd` section is optional and only consulted when the `sia` feature is
//! built. Encryption keys are **not** stored here — they live in the OS keychain
//! (see [`crate::keystore`]) or a `0600` secrets file, per the security model.
//!
//! # Discovery precedence
//!
//! 1. An explicit `--config PATH` (the file must exist).
//! 2. `$XDG_CONFIG_HOME/obsidianlog/config.toml`, if `XDG_CONFIG_HOME` is set.
//! 3. `~/.config/obsidianlog/config.toml` otherwise, where `~` is `$HOME`, or
//!    `%USERPROFILE%` when `HOME` isn't set (the default on Windows).
//!
//! If neither `--config` nor a file at the resolved default path is present,
//! [`Config::load`] falls back to [`Config::default`] rather than erroring —
//! commands should work out of the box against the local backend.
//!
//! # Credential storage format (ADR-0015)
//!
//! `credential_store_version` records which local credential-storage layout
//! the config expects. [`Config::load`] — used by every command except
//! `init` — refuses to load a config whose version doesn't match
//! [`CURRENT_CREDENTIAL_VERSION`], before any keychain access happens: a
//! missing version (a pre-0.2 config) or an unrecognized future one both
//! fail with a specific, actionable message rather than being silently
//! reinterpreted. `init` alone uses [`Config::load_for_init`], which
//! surfaces a legacy config as a distinct outcome instead of erroring, so it
//! can offer a supported route back to a fresh setup. See ADR-0015.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The credential-storage format this build of obsidianlog writes and
/// expects: one bundled keychain item holding the encryption key and an
/// optional Sia app key together (ADR-0015). A config missing this marker
/// predates the bundled format (two independent keychain items); a config
/// carrying a higher number is from a newer, unrecognized build.
pub const CURRENT_CREDENTIAL_VERSION: u32 = 2;

/// Top-level ObsidianLog configuration, persisted as TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Storage bucket / namespace logs are archived under.
    pub bucket: String,

    /// Local (default) storage backend settings.
    pub local: LocalConfig,

    /// How to reach the user's indexd deployment. `None` (the default) uses the
    /// local backend only; set this when archiving to Sia (`sia` feature).
    #[serde(default)]
    pub indexd: Option<IndexdConfig>,

    /// Local HTTP ingest server settings.
    pub serve: ServeConfig,

    /// Chunking / time-window settings.
    pub chunking: ChunkingConfig,

    /// Which local credential-storage layout this config expects (see the
    /// module docs and ADR-0015). `None` when parsed from a file that
    /// predates this field — treated as the legacy, two-item layout.
    /// Explicitly `#[serde(default)]` (defaulting to `None`) rather than
    /// relying on the struct's blanket default, following the same pattern
    /// already used for `indexd` above, so a missing field always means
    /// "legacy" regardless of what `Config::default()`'s own value is.
    #[serde(default)]
    pub credential_store_version: Option<u32>,
}

/// Settings for the default, Sia-free local storage backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalConfig {
    /// Directory the local backend stores chunks, indexes, and manifests under.
    pub data_dir: PathBuf,
}

/// Connection details for the user's indexd gateway (Sia archival).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexdConfig {
    /// Base URL of the indexd HTTP API.
    pub url: String,
    /// Sia bucket / namespace logs are archived under.
    pub bucket: String,
}

/// Settings for the local Vector-compatible ingest endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServeConfig {
    /// Address the ingest server binds to.
    pub bind: String,
}

/// Controls how log batches are grouped into discrete chunk files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChunkingConfig {
    /// Length of each chunk's time window, in seconds (default: 1 hour).
    pub window_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bucket: "obsidianlog".to_string(),
            local: LocalConfig {
                data_dir: PathBuf::from("./obsidianlog-data"),
            },
            // Local-first: no Sia node required by default.
            indexd: None,
            serve: ServeConfig {
                bind: "127.0.0.1:7080".to_string(),
            },
            chunking: ChunkingConfig { window_secs: 3600 },
            // A freshly-created config always writes the current format, so
            // it round-trips through Config::load cleanly from here on.
            credential_store_version: Some(CURRENT_CREDENTIAL_VERSION),
        }
    }
}

impl Default for LocalConfig {
    fn default() -> Self {
        Config::default().local
    }
}

impl Default for ServeConfig {
    fn default() -> Self {
        Config::default().serve
    }
}

impl Default for ChunkingConfig {
    fn default() -> Self {
        Config::default().chunking
    }
}

/// Resolve the default config path from XDG/home environment values.
///
/// Pure function over already-read env vars so it can be unit tested without
/// mutating (or depending on) the real process environment. `userprofile` is
/// the Windows fallback: PowerShell and cmd.exe don't set `HOME` by default
/// (only `USERPROFILE`), so without it every fresh Windows install fails
/// here before `init` gets a chance to run.
fn resolve_default_path(
    xdg_config_home: Option<&str>,
    home: Option<&str>,
    userprofile: Option<&str>,
) -> Result<PathBuf> {
    if let Some(xdg) = xdg_config_home.filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(xdg).join("obsidianlog").join("config.toml"));
    }
    let home = home
        .filter(|s| !s.is_empty())
        .or_else(|| userprofile.filter(|s| !s.is_empty()))
        .context(
            "could not determine the config directory: none of XDG_CONFIG_HOME, HOME, or \
             USERPROFILE is set (pass --config explicitly)",
        )?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("obsidianlog")
        .join("config.toml"))
}

/// Parse `text` (read from `path`, used only for the error message) as a
/// [`Config`], with no credential-format validation — shared by the strict
/// [`Config::load`] and the permissive [`Config::load_for_init`], which
/// apply different validation over the same underlying parse.
fn parse(text: &str, path: &Path) -> Result<Config> {
    toml::from_str(text).with_context(|| format!("parsing config file at {}", path.display()))
}

/// Outcome of [`Config::load_for_init`] — the one place in the codebase
/// allowed to see a legacy config as something other than a hard failure.
pub enum ConfigLoadOutcome {
    /// No config file exists yet at the resolved path — a fresh install.
    Missing,
    /// A config file in the current credential-storage format.
    Current(Config),
    /// A config file predating the credential-storage format marker (no
    /// `credential_store_version` field). Carries the parsed config so
    /// `init` can still inspect its other fields if useful, but its
    /// credential-bearing fields must not be trusted for keychain access.
    Legacy(Config),
}

impl Config {
    /// Resolve the default config path (`$XDG_CONFIG_HOME/obsidianlog/config.toml`,
    /// falling back to `~/.config/obsidianlog/config.toml`, using `USERPROFILE`
    /// in place of `HOME` on Windows when `HOME` itself isn't set).
    pub fn default_path() -> Result<PathBuf> {
        resolve_default_path(
            std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
            std::env::var("HOME").ok().as_deref(),
            std::env::var("USERPROFILE").ok().as_deref(),
        )
    }

    /// Load configuration from `path`, or the default path when `None`.
    ///
    /// An explicit `path` must exist. With no `path`, a missing file at the
    /// resolved default location is not an error — [`Config::default`] is used
    /// instead, so commands work with zero setup against the local backend.
    ///
    /// Every command except `init` must use this function: a config whose
    /// credential-storage format doesn't match [`CURRENT_CREDENTIAL_VERSION`]
    /// (missing — legacy — or newer than this build understands) fails here,
    /// before any keychain access happens. `init` alone uses
    /// [`Config::load_for_init`] instead, since offering a route back to a
    /// fresh setup is specifically its job (see ADR-0015); no other caller
    /// should ever see a legacy config as anything but a hard failure.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        match path {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("reading config file at {}", path.display()))?;
                let config = parse(&text, path)?;
                config.require_supported_credential_store()?;
                Ok(config)
            }
            None => {
                let default_path = Self::default_path()?;
                match std::fs::read_to_string(&default_path) {
                    Ok(text) => {
                        let config = parse(&text, &default_path)?;
                        config.require_supported_credential_store()?;
                        Ok(config)
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
                    Err(e) => Err(e).with_context(|| {
                        format!("reading config file at {}", default_path.display())
                    }),
                }
            }
        }
    }

    /// Load configuration for `init`'s use only. Unlike [`Config::load`], a
    /// missing config file resolves to [`ConfigLoadOutcome::Missing`]
    /// (whether `path` was explicit or defaulted — `init` treats both the
    /// same, since either way it means "nothing to reuse yet") and a legacy
    /// config resolves to [`ConfigLoadOutcome::Legacy`] rather than erroring,
    /// so `init` can print what continuing would abandon and offer a
    /// supported route to a fresh setup. An unrecognized *future* format
    /// still errors here exactly as it does in [`Config::load`] — there's no
    /// reasonable repair action for a config written by a newer build.
    pub fn load_for_init(path: Option<&Path>) -> Result<ConfigLoadOutcome> {
        let resolved = match path {
            Some(p) => p.to_path_buf(),
            None => Self::default_path()?,
        };
        match std::fs::read_to_string(&resolved) {
            Ok(text) => {
                let config = parse(&text, &resolved)?;
                match config.credential_store_version {
                    Some(v) if v == CURRENT_CREDENTIAL_VERSION => {
                        Ok(ConfigLoadOutcome::Current(config))
                    }
                    Some(v) if v > CURRENT_CREDENTIAL_VERSION => {
                        Err(unsupported_future_version_error(v))
                    }
                    _ => Ok(ConfigLoadOutcome::Legacy(config)),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ConfigLoadOutcome::Missing),
            Err(e) => {
                Err(e).with_context(|| format!("reading config file at {}", resolved.display()))
            }
        }
    }

    /// Persist configuration to `path`, or the default path when `None`.
    pub fn save(&self, path: Option<&Path>) -> Result<()> {
        let path = match path {
            Some(path) => path.to_path_buf(),
            None => Self::default_path()?,
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating config directory {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serializing config")?;
        std::fs::write(&path, text)
            .with_context(|| format!("writing config file at {}", path.display()))?;
        Ok(())
    }

    /// The gate behind [`Config::load`]: errors unless
    /// `credential_store_version` matches [`CURRENT_CREDENTIAL_VERSION`]
    /// exactly. See the module docs and ADR-0015.
    fn require_supported_credential_store(&self) -> Result<()> {
        match self.credential_store_version {
            Some(v) if v == CURRENT_CREDENTIAL_VERSION => Ok(()),
            Some(v) if v > CURRENT_CREDENTIAL_VERSION => Err(unsupported_future_version_error(v)),
            _ => anyhow::bail!(
                "this config uses the pre-0.2 credential storage layout (one encryption key \
                 and one Sia app key stored as two separate OS keychain items). obsidianlog \
                 0.2 stores them as a single bundled item instead, and there is no automatic \
                 migration. To keep reading archives created under the old layout, use \
                 obsidianlog v0.1.1. To start fresh under the new layout — this rotates the \
                 encryption key, and previously archived data will no longer be decryptable — \
                 run `obsidianlog init --force`."
            ),
        }
    }
}

fn unsupported_future_version_error(found: u32) -> anyhow::Error {
    anyhow::anyhow!(
        "this config was written by a newer version of obsidianlog (credential storage \
         format {found}, this build only understands up to {CURRENT_CREDENTIAL_VERSION}) — \
         upgrade obsidianlog to use it"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_path_prefers_xdg_config_home() {
        let path = resolve_default_path(Some("/xdg"), Some("/home/user"), None).unwrap();
        assert_eq!(path, PathBuf::from("/xdg/obsidianlog/config.toml"));
    }

    #[test]
    fn default_path_falls_back_to_home_when_xdg_unset() {
        let path = resolve_default_path(None, Some("/home/user"), None).unwrap();
        assert_eq!(
            path,
            PathBuf::from("/home/user/.config/obsidianlog/config.toml")
        );
    }

    #[test]
    fn default_path_falls_back_to_home_when_xdg_empty() {
        let path = resolve_default_path(Some(""), Some("/home/user"), None).unwrap();
        assert_eq!(
            path,
            PathBuf::from("/home/user/.config/obsidianlog/config.toml")
        );
    }

    #[test]
    fn default_path_falls_back_to_userprofile_when_home_unset() {
        // The default state of a fresh Windows shell: HOME isn't set, only
        // USERPROFILE is.
        let path = resolve_default_path(None, None, Some(r"C:\Users\user")).unwrap();
        assert_eq!(
            path,
            PathBuf::from(r"C:\Users\user").join(".config/obsidianlog/config.toml")
        );
    }

    #[test]
    fn default_path_prefers_home_over_userprofile_when_both_set() {
        let path = resolve_default_path(None, Some("/home/user"), Some(r"C:\Users\user")).unwrap();
        assert_eq!(
            path,
            PathBuf::from("/home/user/.config/obsidianlog/config.toml")
        );
    }

    #[test]
    fn default_path_errors_when_none_are_set() {
        assert!(resolve_default_path(None, None, None).is_err());
    }

    #[test]
    fn load_with_explicit_path_parses_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
            bucket = "custom-bucket"
            credential_store_version = 2

            [local]
            data_dir = "/data"

            [serve]
            bind = "0.0.0.0:9000"

            [chunking]
            window_secs = 60
            "#,
        )
        .unwrap();

        let config = Config::load(Some(&path)).unwrap();
        assert_eq!(config.bucket, "custom-bucket");
        assert_eq!(config.local.data_dir, PathBuf::from("/data"));
        assert_eq!(config.serve.bind, "0.0.0.0:9000");
        assert_eq!(config.chunking.window_secs, 60);
        assert!(config.indexd.is_none());
    }

    #[test]
    fn load_with_missing_explicit_path_errors() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist.toml");
        assert!(Config::load(Some(&missing)).is_err());
    }

    #[test]
    fn load_with_partial_file_fills_in_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "bucket = \"only-the-bucket\"\ncredential_store_version = 2\n",
        )
        .unwrap();

        let config = Config::load(Some(&path)).unwrap();
        assert_eq!(config.bucket, "only-the-bucket");
        assert_eq!(config.serve.bind, Config::default().serve.bind);
        assert_eq!(
            config.chunking.window_secs,
            Config::default().chunking.window_secs
        );
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let config = Config {
            bucket: "roundtrip".to_string(),
            ..Config::default()
        };

        config.save(Some(&path)).unwrap();
        let loaded = Config::load(Some(&path)).unwrap();
        assert_eq!(loaded.bucket, "roundtrip");
    }

    #[test]
    fn save_then_load_round_trips_an_indexd_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = Config {
            indexd: Some(IndexdConfig {
                url: "https://indexd.example.com".to_string(),
                bucket: "sia-bucket".to_string(),
            }),
            ..Config::default()
        };

        config.save(Some(&path)).unwrap();
        let loaded = Config::load(Some(&path)).unwrap();
        let indexd = loaded.indexd.expect("indexd section must round-trip");
        assert_eq!(indexd.url, "https://indexd.example.com");
        assert_eq!(indexd.bucket, "sia-bucket");
    }

    #[test]
    fn save_writes_the_current_credential_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Config::default().save(Some(&path)).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("credential_store_version = 2"));
    }

    #[test]
    fn load_rejects_a_legacy_config_missing_the_version_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "bucket = \"legacy\"\n").unwrap();

        let err = Config::load(Some(&path)).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("pre-0.2"), "{message}");
        assert!(message.contains("v0.1.1"), "{message}");
        assert!(message.contains("--force"), "{message}");
    }

    #[test]
    fn load_rejects_an_unrecognized_future_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "bucket = \"future\"\ncredential_store_version = 99\n",
        )
        .unwrap();

        let err = Config::load(Some(&path)).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("newer version"), "{message}");
    }

    #[test]
    fn load_accepts_the_current_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "bucket = \"ok\"\ncredential_store_version = 2\n").unwrap();

        assert!(Config::load(Some(&path)).is_ok());
    }

    #[test]
    fn load_for_init_reports_missing_when_no_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");

        assert!(matches!(
            Config::load_for_init(Some(&path)).unwrap(),
            ConfigLoadOutcome::Missing
        ));
    }

    #[test]
    fn load_for_init_reports_current_for_a_current_format_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "bucket = \"ok\"\ncredential_store_version = 2\n").unwrap();

        assert!(matches!(
            Config::load_for_init(Some(&path)).unwrap(),
            ConfigLoadOutcome::Current(_)
        ));
    }

    #[test]
    fn load_for_init_reports_legacy_for_a_config_missing_the_version_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "bucket = \"legacy\"\n").unwrap();

        match Config::load_for_init(Some(&path)).unwrap() {
            ConfigLoadOutcome::Legacy(config) => assert_eq!(config.bucket, "legacy"),
            _ => panic!("expected Legacy"),
        }
    }

    #[test]
    fn load_for_init_still_rejects_an_unrecognized_future_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "bucket = \"future\"\ncredential_store_version = 99\n",
        )
        .unwrap();

        assert!(Config::load_for_init(Some(&path)).is_err());
    }
}
