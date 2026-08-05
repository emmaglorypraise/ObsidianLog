//! [`AnyBackend`] — a backend chosen at runtime rather than compile time.
//!
//! `ArchiveEngine<B: StorageBackend>` needs one concrete `B` per instance. A
//! caller that only learns which backend to use from config at runtime (the
//! CLI, picking Local vs Sia) can't express that with the generic directly —
//! `AnyBackend` closes that gap by implementing [`StorageBackend`] itself,
//! delegating to whichever concrete backend it wraps, so callers can hold a
//! single `ArchiveEngine<AnyBackend>` regardless of which one was chosen.

use async_trait::async_trait;

use obsidianlog_core::backend::{StorageBackend, TimeRange};
use obsidianlog_core::chunk::{Chunk, ChunkRef};
use obsidianlog_core::error::Result;
use obsidianlog_core::index::ServiceWindowIndex;
use obsidianlog_core::manifest::Manifest;

use super::LocalBackend;
#[cfg(feature = "sia")]
use super::SiaBackend;

/// A runtime-selected backend. The `Sia` variant only exists when the `sia`
/// feature is enabled, so a build without it can't even construct one.
pub enum AnyBackend {
    Local(LocalBackend),
    #[cfg(feature = "sia")]
    Sia(SiaBackend),
}

impl From<LocalBackend> for AnyBackend {
    fn from(backend: LocalBackend) -> Self {
        Self::Local(backend)
    }
}

#[cfg(feature = "sia")]
impl From<SiaBackend> for AnyBackend {
    fn from(backend: SiaBackend) -> Self {
        Self::Sia(backend)
    }
}

#[async_trait]
impl StorageBackend for AnyBackend {
    async fn put_archive(&self, chunk: &Chunk, index: &ServiceWindowIndex) -> Result<()> {
        match self {
            Self::Local(b) => b.put_archive(chunk, index).await,
            #[cfg(feature = "sia")]
            Self::Sia(b) => b.put_archive(chunk, index).await,
        }
    }

    async fn get_chunk(&self, service: &str, window: &str, sequence: u64) -> Result<Chunk> {
        match self {
            Self::Local(b) => b.get_chunk(service, window, sequence).await,
            #[cfg(feature = "sia")]
            Self::Sia(b) => b.get_chunk(service, window, sequence).await,
        }
    }

    async fn get_index(
        &self,
        service: &str,
        window: &str,
        sequence: u64,
    ) -> Result<ServiceWindowIndex> {
        match self {
            Self::Local(b) => b.get_index(service, window, sequence).await,
            #[cfg(feature = "sia")]
            Self::Sia(b) => b.get_index(service, window, sequence).await,
        }
    }

    async fn list_chunks(&self, service: &str, range: Option<TimeRange>) -> Result<Vec<ChunkRef>> {
        match self {
            Self::Local(b) => b.list_chunks(service, range).await,
            #[cfg(feature = "sia")]
            Self::Sia(b) => b.list_chunks(service, range).await,
        }
    }

    async fn read_manifest(&self) -> Result<Manifest> {
        match self {
            Self::Local(b) => b.read_manifest().await,
            #[cfg(feature = "sia")]
            Self::Sia(b) => b.read_manifest().await,
        }
    }

    async fn write_manifest(&self, manifest: &Manifest) -> Result<()> {
        match self {
            Self::Local(b) => b.write_manifest(manifest).await,
            #[cfg(feature = "sia")]
            Self::Sia(b) => b.write_manifest(manifest).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use obsidianlog_core::manifest::ManifestServiceChain;

    #[tokio::test]
    async fn local_variant_delegates_every_method() {
        let dir = tempfile::tempdir().unwrap();
        let backend: AnyBackend = LocalBackend::new(dir.path(), "obsidianlog").into();

        let mut manifest = Manifest::new("obsidianlog");
        manifest
            .services
            .insert("api".to_string(), ManifestServiceChain::new("api", 0));
        backend.write_manifest(&manifest).await.unwrap();

        let fetched = backend.read_manifest().await.unwrap();
        assert_eq!(fetched, manifest);

        assert!(backend.list_chunks("api", None).await.unwrap().is_empty());
        assert!(matches!(
            backend.get_chunk("api", "no-such-window", 0).await,
            Err(obsidianlog_core::Error::NotFound(_))
        ));
    }
}
