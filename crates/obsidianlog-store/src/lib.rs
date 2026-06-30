//! ObsidianLog core storage library.
//!
//! This crate owns the deterministic processing pipeline that every log batch
//! passes through before it touches durable storage, over a pluggable backend.
//!
//! Pipeline order (see the project architecture):
//! [`parse`] (raw Vector events → records) → [`chunking`] (group into
//! per-`(service, window)` buckets) → [`compress`] → [`encrypt`] → [`chain`]
//! (hash-link per service), with [`index`] building the lightweight metadata
//! queries hit first.
//!
//! The durable-storage integration is isolated behind the
//! [`backend::StorageBackend`] trait (defined in `obsidianlog-core`). The
//! default [`backend::LocalBackend`] needs no Sia node, so the whole pipeline
//! builds and tests offline; the real Sia integration lives in a
//! `sia`-feature-gated [`backend::SiaBackend`] and never touches the
//! crypto/chunking code (see ADR-0004).
//!
//! The shared data model (`Chunk`, `Manifest`, `ServiceWindowIndex`, …), the
//! [`Error`]/[`Result`] type, and the [`backend::StorageBackend`] trait live in
//! `obsidianlog-core`; this crate implements the behavior over them.

pub mod backend;
pub mod chain;
pub mod chunking;
pub mod compress;
pub mod encrypt;
pub mod error;
pub mod index;
pub mod parse;

pub use error::{Error, Result};
