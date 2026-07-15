//! Configuration model and loading.
//!
//! ObsidianLog is self-hosted: configuration captures where logs are archived
//! and how chunks are sized. It is **local-first** — the default backend writes
//! to a local directory and needs no Sia node (mock-first invariant). The
//! `indexd` section is optional and only consulted when the `sia` feature is
//! built. Encryption keys are **not** stored here — they live in the OS keychain
//! (see [`crate::keystore`]) or a `0600` secrets file, per the security model.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Top-level ObsidianLog configuration, persisted as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Local (default) storage backend settings.
    pub local: LocalConfig,

    /// How to reach the user's indexd deployment. `None` (the default) uses the
    /// local backend only; set this when archiving to Sia (`sia` feature).
    #[serde(default)]
    pub indexd: Option<IndexdConfig>,

    /// Local HTTP ingest server settings.
    pub ingest: IngestConfig,

    /// Chunking / time-window settings.
    pub chunking: ChunkingConfig,
}

/// Settings for the default, Sia-free local storage backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub struct IngestConfig {
    /// Address the ingest server binds to.
    pub bind: String,
}

/// Controls how log batches are grouped into discrete chunk files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkingConfig {
    /// Length of each chunk's time window, in seconds (default: 1 hour).
    pub window_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            local: LocalConfig {
                data_dir: PathBuf::from("./obsidianlog-data"),
            },
            // Local-first: no Sia node required by default.
            indexd: None,
            ingest: IngestConfig {
                bind: "127.0.0.1:7080".to_string(),
            },
            chunking: ChunkingConfig { window_secs: 3600 },
        }
    }
}

impl Config {
    /// Resolve the default config path (platform config dir).
    ///
    /// TODO(impl): use the platform config directory
    /// (e.g. `~/.config/obsidianlog/config.json`).
    pub fn default_path() -> Result<PathBuf> {
        anyhow::bail!("Config::default_path is not yet implemented")
    }

    /// Load configuration from `path`, or the default path when `None`.
    ///
    /// TODO(impl): read the file, deserialize, and validate (reachable indexd
    /// URL, non-empty bucket, sane window).
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let _ = path;
        anyhow::bail!("Config::load is not yet implemented")
    }

    /// Persist configuration to `path`, or the default path when `None`.
    ///
    /// TODO(impl): serialize as pretty JSON and write atomically.
    pub fn save(&self, path: Option<&Path>) -> Result<()> {
        let _ = path;
        anyhow::bail!("Config::save is not yet implemented")
    }
}
