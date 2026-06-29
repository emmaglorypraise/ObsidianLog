//! ObsidianLog core storage library.
//!
//! This crate owns the deterministic processing pipeline that every log batch
//! passes through before it touches Sia, plus the append-only chunk store and
//! the lightweight metadata index that queries hit first.
//!
//! Pipeline order (see the project architecture):
//! [`compress`] → [`encrypt`] → [`hashchain`] → [`chunk`], with
//! [`index`] and [`manifest`] tracking metadata and the chain head.
//!
//! The durable-storage integration is isolated behind the
//! [`backend::StorageBackend`] trait (defined in `obsidianlog-core`). The
//! default [`backend::LocalBackend`] needs no Sia node, so the whole pipeline
//! builds and tests offline; the real Sia integration lives in a
//! `sia`-feature-gated [`backend::SiaBackend`] and never touches the
//! crypto/chunking code (see ADR-0004).
//!
//! Shared vocabulary (the [`Error`]/[`Result`] type, the
//! [`backend::StorageBackend`] trait, and value types like
//! [`chunk::ChunkId`]) is owned by `obsidianlog-core` and re-exported from its
//! semantic home here so call sites read naturally.
//!
//! # Status
//!
//! Scaffold. The module surface is final; every operation is a `todo!()` with a
//! `TODO(impl)` note describing the intended behavior.

pub mod backend;
pub mod chunk;
pub mod compress;
pub mod encrypt;
pub mod error;
pub mod hashchain;
pub mod index;
pub mod manifest;

pub use error::{Error, Result};
