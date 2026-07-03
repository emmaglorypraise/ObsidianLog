//! Ingest server configuration.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};

/// Ingest server configuration, loaded from a TOML file or defaults.
///
/// The encryption key is **not** read from the config file — it is set by the
/// caller (e.g. the CLI, from the OS keychain) so key material never lives in a
/// plaintext config. It defaults to all-zero, which [`crate::serve`] warns about.
#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Address to bind the HTTP endpoint to.
    pub bind: String,
    /// Storage bucket / namespace.
    pub bucket: String,
    /// Directory the local backend writes chunks/index/manifest under.
    pub storage_root: PathBuf,
    /// Chunk time-window length, in seconds.
    pub window_secs: u64,
    /// AES-256 key. Never loaded from the config file; set by the caller.
    #[serde(skip)]
    pub encryption_key: [u8; 32],
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:7080".to_string(),
            bucket: "obsidianlog".to_string(),
            storage_root: PathBuf::from("./obsidianlog-data"),
            window_secs: 3600,
            encryption_key: [0u8; 32],
        }
    }
}

impl Config {
    /// Load config from `path`, or return defaults when `path` is `None`.
    ///
    /// Missing fields fall back to defaults; the encryption key is always set by
    /// the caller regardless of the file.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        match path {
            Some(path) => {
                let text = std::fs::read_to_string(path)?;
                toml::from_str(&text).map_err(|e| Error::Config(e.to_string()))
            }
            None => Ok(Self::default()),
        }
    }
}
