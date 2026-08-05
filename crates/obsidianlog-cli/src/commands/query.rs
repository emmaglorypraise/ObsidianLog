//! `obsidianlog query` — retrieve archived logs.
//!
//! Resolves `--from`/`--to` (RFC 3339 or relative durations like `24h`) and the
//! remaining filters into an [`IndexQuery`], runs it through
//! [`ArchiveEngine::query`]'s index-first algorithm, and renders the result in
//! the requested format.

use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use obsidianlog_core::record::LogRecord;
use obsidianlog_store::ArchiveEngine;
use obsidianlog_store::encrypt::EncryptionKey;
use obsidianlog_store::index::IndexQuery;

use crate::backend::resolve_backend;
use crate::cli::{OutputFormat, QueryArgs};
use crate::config::Config;
use crate::keystore;

/// Execute a query and render results to stdout.
pub fn run(args: QueryArgs, config_path: Option<PathBuf>) -> Result<()> {
    let config = Config::load(config_path.as_deref())?;
    let key = EncryptionKey::new(
        keystore::default_encryption_key_store()?
            .load()
            .context("loading the encryption key (run `obsidianlog init` first)")?,
    );

    let now = Utc::now();
    let since = args
        .from
        .as_deref()
        .map(|s| parse_time(s, now))
        .transpose()?;
    let until = args.to.as_deref().map(|s| parse_time(s, now)).transpose()?;

    let index_query = IndexQuery {
        since,
        until,
        service: args.service,
        level: args.level,
        host: args.host,
        keyword: args.keyword,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting the query runtime")?;

    let backend = runtime.block_on(resolve_backend(&config))?;
    let engine = ArchiveEngine::new(backend, key, config.bucket.clone())
        .with_window_secs(config.chunking.window_secs);

    let mut records = runtime
        .block_on(engine.query(&index_query))
        .context("running the query")?;
    records.sort_by_key(|r| r.timestamp);

    render(&records, args.format)
}

/// Parse `input` as RFC 3339, or as a relative duration (e.g. `24h`) before
/// `now`.
fn parse_time(input: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(input) {
        return Ok(dt.with_timezone(&Utc));
    }
    let duration = parse_relative_duration(input).with_context(|| {
        format!("invalid time value {input:?}: expected RFC 3339 (e.g. 2026-06-29T15:00:00Z) or a relative duration (e.g. 24h, 30m, 7d)")
    })?;
    Ok(now - duration)
}

/// Parse a relative duration like `24h`, `30m`, `7d`: an integer followed by
/// one of `s`/`m`/`h`/`d`/`w`.
fn parse_relative_duration(input: &str) -> Option<Duration> {
    let input = input.trim();
    let split_at = input.len().checked_sub(1)?;
    let (number, unit) = input.split_at(split_at);
    let amount: i64 = number.parse().ok()?;
    match unit {
        "s" => Some(Duration::seconds(amount)),
        "m" => Some(Duration::minutes(amount)),
        "h" => Some(Duration::hours(amount)),
        "d" => Some(Duration::days(amount)),
        "w" => Some(Duration::weeks(amount)),
        _ => None,
    }
}

fn render(records: &[LogRecord], format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Human => render_human(records),
        OutputFormat::Json => render_json(records),
        OutputFormat::Raw => render_raw(records),
    }
}

fn render_human(records: &[LogRecord]) -> Result<()> {
    let colorize = std::io::stdout().is_terminal();
    let service_width = records
        .iter()
        .map(|r| r.service.len())
        .max()
        .unwrap_or(0)
        .max("SERVICE".len());
    let level_width = records
        .iter()
        .map(|r| r.level.as_deref().unwrap_or("-").len())
        .max()
        .unwrap_or(0)
        .max("LEVEL".len());
    let host_width = records
        .iter()
        .map(|r| r.host.as_deref().unwrap_or("-").len())
        .max()
        .unwrap_or(0)
        .max("HOST".len());

    for record in records {
        let timestamp = record
            .timestamp
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let service = &record.service;
        let level_text = record.level.as_deref().unwrap_or("-");
        let host = record.host.as_deref().unwrap_or("-");
        let message = human_message(&record.raw);

        let level = if colorize {
            format!(
                "{}{level_text:<level_width$}{}",
                level_color(record.level.as_deref()),
                RESET
            )
        } else {
            format!("{level_text:<level_width$}")
        };

        println!("{timestamp}  {service:<service_width$}  {level}  {host:<host_width$}  {message}");
    }
    Ok(())
}

fn render_json(records: &[LogRecord]) -> Result<()> {
    let text = serde_json::to_string_pretty(records).context("serializing query results")?;
    println!("{text}");
    Ok(())
}

fn render_raw(records: &[LogRecord]) -> Result<()> {
    for record in records {
        let line = serde_json::to_string(&record.raw).context("serializing a record")?;
        println!("{line}");
    }
    Ok(())
}

/// Best-effort human-readable message: a `msg`/`message` string field if
/// present, else the record's compact raw JSON.
fn human_message(raw: &serde_json::Value) -> String {
    raw.get("msg")
        .or_else(|| raw.get("message"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| raw.to_string())
}

const RESET: &str = "\x1b[0m";

fn level_color(level: Option<&str>) -> &'static str {
    match level.map(str::to_ascii_lowercase).as_deref() {
        Some("error") | Some("fatal") | Some("critical") => "\x1b[31m", // red
        Some("warn") | Some("warning") => "\x1b[33m",                   // yellow
        Some("info") => "\x1b[32m",                                     // green
        Some("debug") | Some("trace") => "\x1b[36m",                    // cyan
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-28T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn parse_time_accepts_rfc3339() {
        let parsed = parse_time("2026-06-29T15:00:00Z", now()).unwrap();
        assert_eq!(
            parsed,
            DateTime::parse_from_rfc3339("2026-06-29T15:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn parse_time_accepts_relative_durations() {
        assert_eq!(
            parse_time("24h", now()).unwrap(),
            now() - Duration::hours(24)
        );
        assert_eq!(
            parse_time("30m", now()).unwrap(),
            now() - Duration::minutes(30)
        );
        assert_eq!(parse_time("7d", now()).unwrap(), now() - Duration::days(7));
        assert_eq!(parse_time("2w", now()).unwrap(), now() - Duration::weeks(2));
        assert_eq!(
            parse_time("45s", now()).unwrap(),
            now() - Duration::seconds(45)
        );
    }

    #[test]
    fn parse_time_rejects_garbage() {
        assert!(parse_time("not-a-time", now()).is_err());
        assert!(parse_time("24x", now()).is_err());
        assert!(parse_time("", now()).is_err());
    }

    #[test]
    fn human_message_prefers_msg_then_message_then_falls_back_to_raw() {
        assert_eq!(
            human_message(&serde_json::json!({ "msg": "hello" })),
            "hello"
        );
        assert_eq!(
            human_message(&serde_json::json!({ "message": "world" })),
            "world"
        );
        assert_eq!(
            human_message(&serde_json::json!({ "id": 1 })),
            serde_json::json!({ "id": 1 }).to_string()
        );
    }
}
