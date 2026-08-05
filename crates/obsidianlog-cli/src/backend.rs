//! Runtime backend resolution: build the [`AnyBackend`] a command should
//! archive/query/verify against, based on `config.indexd` — [`LocalBackend`]
//! when unset, the Sia backend (with the `sia` feature) when set.

use anyhow::Result;

use obsidianlog_store::backend::{AnyBackend, LocalBackend};

use crate::config::Config;

/// Resolve the backend for `config`: `LocalBackend` if `config.indexd` is
/// unset, otherwise the Sia backend — connecting to the configured indexer
/// using the app key `obsidianlog init` stored.
///
/// Without the `sia` feature compiled in, a Sia-configured install errors
/// clearly here instead of silently falling back to local.
pub async fn resolve_backend(config: &Config) -> Result<AnyBackend> {
    match &config.indexd {
        Some(indexd) => connect_sia(indexd).await,
        None => Ok(LocalBackend::new(&config.local.data_dir, &config.bucket).into()),
    }
}

#[cfg(feature = "sia")]
async fn connect_sia(indexd: &crate::config::IndexdConfig) -> Result<AnyBackend> {
    use anyhow::Context;
    use obsidianlog_store::backend::{SiaBackend, SiaConfig};

    let app_key = crate::keystore::default_sia_app_key_store()?
        .load()
        .context("loading the Sia app key (run `obsidianlog init` and choose the sia backend)")?;

    let backend = SiaBackend::connect(SiaConfig {
        indexer_url: indexd.url.clone(),
        bucket: indexd.bucket.clone(),
        app_key,
    })
    .await
    .context("connecting to the Sia indexer")?;

    Ok(backend.into())
}

#[cfg(not(feature = "sia"))]
async fn connect_sia(_indexd: &crate::config::IndexdConfig) -> Result<AnyBackend> {
    anyhow::bail!(
        "config specifies a Sia indexer, but this build of obsidianlog was compiled without \
         Sia support; rebuild with `cargo build --features sia`"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use obsidianlog_store::backend::StorageBackend;

    #[tokio::test]
    async fn resolves_local_backend_when_indexd_is_unset() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            local: crate::config::LocalConfig {
                data_dir: dir.path().to_path_buf(),
            },
            ..Config::default()
        };

        let backend = resolve_backend(&config).await.unwrap();
        assert!(matches!(backend, AnyBackend::Local(_)));
        // Sanity: it's actually usable, not just the right variant.
        assert!(backend.read_manifest().await.is_err());
    }

    #[cfg(not(feature = "sia"))]
    #[tokio::test]
    async fn errors_clearly_when_indexd_is_set_without_the_sia_feature() {
        let config = Config {
            indexd: Some(crate::config::IndexdConfig {
                url: "https://indexd.example.com".to_string(),
                bucket: "obsidianlog".to_string(),
            }),
            ..Config::default()
        };

        // AnyBackend isn't Debug (SiaBackend wraps an SDK handle that isn't
        // either), so match instead of unwrap_err().
        match resolve_backend(&config).await {
            Ok(_) => panic!("expected an error when indexd is set without the sia feature"),
            Err(err) => assert!(err.to_string().contains("--features sia"), "{err}"),
        }
    }
}
