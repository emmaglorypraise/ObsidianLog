//! Integration tests for the storage pipeline.
//!
//! These exercise the public surface of `obsidianlog-store` end to end. They are
//! `#[ignore]`d until the pipeline is implemented so CI stays green on the
//! scaffold while still tracking the intended behavior.

use obsidianlog_core::types::ChunkId;

#[test]
fn chunk_paths_follow_on_sia_layout() {
    // This one needs no pipeline logic, so it runs today and locks the layout.
    let id = ChunkId {
        service: "api".to_string(),
        window: "2026-06-22-15".to_string(),
    };
    assert_eq!(id.chunk_path(), "chunks/api/2026-06-22-15.bin");
    assert_eq!(id.index_path(), "index/api/2026-06-22-15.idx");
}

#[test]
#[ignore = "TODO(impl): implement the compress -> encrypt -> chain pipeline"]
fn roundtrip_preserves_log_batch() {
    // TODO(impl): compress + encrypt a batch, then decrypt + decompress and
    // assert the bytes round-trip exactly.
}

#[test]
#[ignore = "TODO(impl): implement hash-chain verification"]
fn tampered_chunk_breaks_the_chain() {
    // TODO(impl): build a short chain, mutate one chunk, and assert `verify`
    // reports the break at the expected position.
}
