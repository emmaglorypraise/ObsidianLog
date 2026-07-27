//! CLI surface tests.
//!
//! Verifies the argument parser stays wired correctly. These run today (they
//! don't touch the stubbed command bodies) and guard against accidental CLI
//! regressions.

use clap::CommandFactory;
use clap::error::ErrorKind;

use obsidianlog_cli::Cli;

#[test]
fn cli_definition_is_valid() {
    // Panics if the derive tree is malformed (duplicate args, bad defaults, …).
    Cli::command().debug_assert();
}

#[test]
fn top_level_help_renders() {
    let err = Cli::command()
        .try_get_matches_from(["obsidianlog", "--help"])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    assert!(!err.to_string().is_empty());
}

#[test]
fn top_level_version_renders() {
    let err = Cli::command()
        .try_get_matches_from(["obsidianlog", "--version"])
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::DisplayVersion);
    assert!(err.to_string().contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn each_subcommand_help_renders() {
    for subcommand in ["init", "serve", "query", "verify"] {
        let err = Cli::command()
            .try_get_matches_from(["obsidianlog", subcommand, "--help"])
            .unwrap_err();
        assert_eq!(
            err.kind(),
            ErrorKind::DisplayHelp,
            "obsidianlog {subcommand} --help should render help text"
        );
        assert!(!err.to_string().is_empty());
    }
}
