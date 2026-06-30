//! Storage backends.
//!
//! The [`StorageBackend`] trait is defined in `obsidianlog-core` and re-exported
//! here. Implementations live in submodules:
//!
//! - [`local`] — [`LocalBackend`], a filesystem-backed store that is the
//!   **default**. The whole pipeline builds, runs, and tests against it with no
//!   Sia node — the mock-first invariant (see CLAUDE.md / ADR-0004).
//! - [`sia`] — [`SiaBackend`], the real Sia integration. Compiled only with the
//!   `sia` feature so the pre-1.0 Sia SDK never enters a default build.
//!
//! Backends are append-only: written chunks are never modified or deleted
//! post-write, and every write is made durable before it returns `Ok`.

pub use obsidianlog_core::backend::StorageBackend;

pub mod local;
pub use local::LocalBackend;

#[cfg(feature = "sia")]
pub mod sia;
#[cfg(feature = "sia")]
pub use sia::SiaBackend;
