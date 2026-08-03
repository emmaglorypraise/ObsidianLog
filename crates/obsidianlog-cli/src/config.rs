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
//! 3. `~/.config/obsidianlog/config.toml` otherwise.
//!
//! If neither `--config` nor a file at the resolved default path is present,
//! [`Config::load`] falls back to [`Config::default`] rather than erroring —
//! commands should work out of the box against the local backend.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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
/// mutating (or depending on) the real process environment.
fn resolve_default_path(xdg_config_home: Option<&str>, home: Option<&str>) -> Result<PathBuf> {
    if let Some(xdg) = xdg_config_home.filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(xdg).join("obsidianlog").join("config.toml"));
    }
    let home = home.context(
        "could not determine the config directory: neither XDG_CONFIG_HOME nor HOME is set \
         (pass --config explicitly)",
    )?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("obsidianlog")
        .join("config.toml"))
}

impl Config {
    /// Resolve the default config path (`$XDG_CONFIG_HOME/obsidianlog/config.toml`,
    /// falling back to `~/.config/obsidianlog/config.toml`).
    pub fn default_path() -> Result<PathBuf> {
        resolve_default_path(
            std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
            std::env::var("HOME").ok().as_deref(),
        )
    }

    /// Load configuration from `path`, or the default path when `None`.
    ///
    /// An explicit `path` must exist. With no `path`, a missing file at the
    /// resolved default location is not an error — [`Config::default`] is used
    /// instead, so commands work with zero setup against the local backend.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        match path {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("reading config file at {}", path.display()))?;
                toml::from_str(&text)
                    .with_context(|| format!("parsing config file at {}", path.display()))
            }
            None => {
                let default_path = Self::default_path()?;
                match std::fs::read_to_string(&default_path) {
                    Ok(text) => toml::from_str(&text).with_context(|| {
                        format!("parsing config file at {}", default_path.display())
                    }),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
                    Err(e) => Err(e).with_context(|| {
                        format!("reading config file at {}", default_path.display())
                    }),
                }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_path_prefers_xdg_config_home() {
        let path = resolve_default_path(Some("/xdg"), Some("/home/user")).unwrap();
        assert_eq!(path, PathBuf::from("/xdg/obsidianlog/config.toml"));
    }

    #[test]
    fn default_path_falls_back_to_home_when_xdg_unset() {
        let path = resolve_default_path(None, Some("/home/user")).unwrap();
        assert_eq!(
            path,
            PathBuf::from("/home/user/.config/obsidianlog/config.toml")
        );
    }

    #[test]
    fn default_path_falls_back_to_home_when_xdg_empty() {
        let path = resolve_default_path(Some(""), Some("/home/user")).unwrap();
        assert_eq!(
            path,
            PathBuf::from("/home/user/.config/obsidianlog/config.toml")
        );
    }

    #[test]
    fn default_path_errors_when_neither_is_set() {
        assert!(resolve_default_path(None, None).is_err());
    }

    #[test]
    fn load_with_explicit_path_parses_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
            bucket = "custom-bucket"

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
        std::fs::write(&path, r#"bucket = "only-the-bucket""#).unwrap();

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
}
