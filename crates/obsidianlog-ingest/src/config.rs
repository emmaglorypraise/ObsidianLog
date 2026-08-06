//! Ingest server configuration.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use obsidianlog_store::encrypt::EncryptionKey;

use crate::error::{Error, Result};

/// Environment variable holding the encryption key directly, as 64 hex chars.
pub const ENCRYPTION_KEY_ENV: &str = "OBSIDIANLOG_ENCRYPTION_KEY";
/// Environment variable holding the *path* to a file containing that same hex
/// key — the Docker/Postgres `_FILE` convention, for mounted secrets.
pub const ENCRYPTION_KEY_FILE_ENV: &str = "OBSIDIANLOG_ENCRYPTION_KEY_FILE";

/// Ingest server configuration, loaded from a TOML file or defaults.
///
/// The encryption key is **not** read from the config file — key material must
/// never live in a plaintext config. It defaults to all-zero; [`crate::serve`]
/// refuses to start with that placeholder. The standalone `obsidianlog-ingest`
/// binary supplies a real key via [`encryption_key_from_env`]; the CLI's
/// `obsidianlog serve` sets it directly from the OS keychain instead.
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
    pub encryption_key: EncryptionKey,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:7080".to_string(),
            bucket: "obsidianlog".to_string(),
            storage_root: PathBuf::from("./obsidianlog-data"),
            window_secs: 3600,
            encryption_key: EncryptionKey::new([0u8; 32]),
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

/// Load the standalone binary's encryption key from the environment.
///
/// Checks [`ENCRYPTION_KEY_FILE_ENV`] first (a path to a file containing the
/// hex key — the convention for mounted Docker/Kubernetes secrets, so the key
/// never sits directly in the process environment), then falls back to
/// [`ENCRYPTION_KEY_ENV`] (the hex key itself, for simpler/dev setups). Errors
/// with an actionable message if neither is set or the value isn't a valid key.
pub fn encryption_key_from_env() -> Result<EncryptionKey> {
    if let Ok(path) = std::env::var(ENCRYPTION_KEY_FILE_ENV) {
        let text = std::fs::read_to_string(&path).map_err(|e| {
            Error::Config(format!(
                "{ENCRYPTION_KEY_FILE_ENV} points at {path:?}, but it could not be read: {e}"
            ))
        })?;
        let bytes = parse_encryption_key_hex(text.trim())
            .map_err(|e| Error::Config(format!("key file at {path:?} is not a valid key: {e}")))?;
        return Ok(EncryptionKey::new(bytes));
    }

    let hex = std::env::var(ENCRYPTION_KEY_ENV).map_err(|_| {
        Error::Config(format!(
            "refusing to start with no encryption key: set {ENCRYPTION_KEY_FILE_ENV} (path to a \
             file holding a 64-character hex key) or {ENCRYPTION_KEY_ENV} (the hex key itself), \
             or use `obsidianlog serve` instead, which loads the key from the OS keychain"
        ))
    })?;
    let bytes = parse_encryption_key_hex(hex.trim())
        .map_err(|e| Error::Config(format!("{ENCRYPTION_KEY_ENV} is not a valid key: {e}")))?;
    Ok(EncryptionKey::new(bytes))
}

/// Decode a 64-character hex string into a 32-byte key.
///
/// Pure function over an already-read value, so it can be unit tested without
/// mutating (or depending on) the real process environment.
fn parse_encryption_key_hex(hex: &str) -> std::result::Result<[u8; 32], String> {
    if hex.len() != 64 {
        return Err(format!("expected 64 hex characters, got {}", hex.len()));
    }
    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        let pair = &hex[i * 2..i * 2 + 2];
        *byte = u8::from_str_radix(pair, 16).map_err(|_| format!("invalid hex byte {pair:?}"))?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex32(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    #[test]
    fn parse_encryption_key_hex_decodes_every_byte_value() {
        let hex: String = (0u8..32).map(|b| format!("{b:02x}")).collect();
        let bytes = parse_encryption_key_hex(&hex).unwrap();
        assert_eq!(bytes, std::array::from_fn(|i| i as u8));
    }

    #[test]
    fn parse_encryption_key_hex_rejects_the_wrong_length() {
        assert!(parse_encryption_key_hex("abcd").is_err());
    }

    #[test]
    fn parse_encryption_key_hex_rejects_non_hex_characters() {
        assert!(parse_encryption_key_hex(&"zz".repeat(32)).is_err());
    }

    #[test]
    fn parse_encryption_key_hex_round_trips_a_uniform_key() {
        let bytes = parse_encryption_key_hex(&hex32(0xAB)).unwrap();
        assert_eq!(bytes, [0xABu8; 32]);
    }
}
