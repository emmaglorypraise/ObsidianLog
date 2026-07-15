//! One-time app onboarding against a Sia indexer (default `https://sia.storage`).
//!
//! Runs the indexer approval flow to derive and export this app's `AppKey` — the
//! per-user credential the Sia backend (and the `--features sia` integration
//! test) authenticate with via `OBSIDIANLOG_APP_KEY`. Compiled only with the
//! `sia` feature.
//!
//! ## Usage
//! ```text
//! cargo run -p obsidianlog-store --features sia --example onboard [-- <indexer-url>]
//! ```
//! You are prompted to paste your Sia recovery phrase (read from stdin, so it
//! never lands in argv, the environment, or shell history — and it is never
//! logged or stored). The tool then prints an approval URL to open in your Sia
//! account; once you approve, it prints your 64-hex `AppKey`. Save it:
//! ```text
//! export OBSIDIANLOG_INDEXD_URL=https://sia.storage
//! export OBSIDIANLOG_APP_KEY=<printed-hex>
//! ```
//!
//! The recovery phrase is the master key to the data; the derived `AppKey` is
//! what should be persisted (keychain / `0600` file), never the phrase itself.

use std::io::{self, BufRead, Write};

use obsidianlog_store::backend::sia::APP_META;
use sia_storage::Builder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Indexer URL: CLI arg, then env, then the hosted default.
    let indexer_url = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("OBSIDIANLOG_INDEXD_URL").ok())
        .unwrap_or_else(|| "https://sia.storage".to_string());

    // Read the recovery phrase from stdin so it stays off argv/env/history.
    eprint!("Paste your Sia recovery phrase, then press Enter: ");
    io::stderr().flush()?;
    let mut mnemonic = String::new();
    io::stdin().lock().read_line(&mut mnemonic)?;
    let mnemonic = mnemonic.trim().to_string();
    if mnemonic.is_empty() {
        return Err("no recovery phrase provided".into());
    }

    eprintln!(
        "\nConnecting to indexer {indexer_url} as app \"{}\"...",
        APP_META.name
    );
    let requesting = Builder::new(indexer_url.as_str(), APP_META)?
        .request_connection()
        .await?;

    eprintln!("\nOpen this URL and approve the app in your Sia account:\n");
    eprintln!("    {}\n", requesting.response_url());
    eprintln!("Waiting for approval (this blocks until you approve)...");

    let sdk = requesting
        .wait_for_approval()
        .await?
        .register(&mnemonic)
        .await?;
    let hex: String = sdk
        .app_key()
        .export()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    eprintln!("\nApproved. Your AppKey (64 hex) is below — save it securely:\n");
    println!("{hex}");
    eprintln!("\nThen run the Sia integration test with:");
    eprintln!("  export OBSIDIANLOG_INDEXD_URL={indexer_url}");
    eprintln!("  export OBSIDIANLOG_APP_KEY={hex}");
    eprintln!("  cargo test -p obsidianlog-store --features sia --test sia -- --nocapture");
    Ok(())
}
