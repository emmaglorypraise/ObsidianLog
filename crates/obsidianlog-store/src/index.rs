//! The lightweight per-`(service, window)` metadata index.
//!
//! For each chunk we build a [`ServiceWindowIndex`] summarizing its records — the
//! time span, the set of levels and hosts, and a set of lowercased keyword
//! tokens. Queries scan these compact summaries first (the target is `<1%` of raw
//! log size) and only fetch and decrypt a chunk when its window *could* contain
//! matches, decided by [`might_match`].

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde_json::Value;

use obsidianlog_core::chunk::ChunkRef;
use obsidianlog_core::index::ServiceWindowIndex;
use obsidianlog_core::record::LogRecord;

/// The filters a query applies against the metadata index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexQuery {
    /// Inclusive lower time bound.
    pub since: Option<DateTime<Utc>>,
    /// Inclusive upper time bound.
    pub until: Option<DateTime<Utc>>,
    /// Restrict to a service.
    pub service: Option<String>,
    /// Restrict to a log level.
    pub level: Option<String>,
    /// Restrict to a host.
    pub host: Option<String>,
    /// Free-text keyword, matched as lowercased word tokens.
    pub keyword: Option<String>,
}

/// Build the metadata index for a chunk's `records`.
///
/// Callers pass the records of a single `(service, window)` chunk; the `chunk`
/// reference is stored so queries can locate the data.
pub fn build_index(chunk: ChunkRef, records: &[LogRecord]) -> ServiceWindowIndex {
    let mut levels = BTreeSet::new();
    let mut hosts = BTreeSet::new();
    let mut keywords = BTreeSet::new();
    let mut min_ts: Option<DateTime<Utc>> = None;
    let mut max_ts: Option<DateTime<Utc>> = None;

    for record in records {
        min_ts = Some(min_ts.map_or(record.timestamp, |m| m.min(record.timestamp)));
        max_ts = Some(max_ts.map_or(record.timestamp, |m| m.max(record.timestamp)));
        if let Some(level) = &record.level {
            levels.insert(level.clone());
        }
        if let Some(host) = &record.host {
            hosts.insert(host.clone());
        }
        collect_tokens(&record.raw, &mut keywords);
    }

    let epoch = DateTime::<Utc>::from_timestamp(0, 0).expect("the Unix epoch is valid");
    ServiceWindowIndex {
        service: chunk.service.clone(),
        window: chunk.window.clone(),
        min_timestamp: min_ts.unwrap_or(epoch),
        max_timestamp: max_ts.unwrap_or(epoch),
        levels,
        hosts,
        keywords,
        chunk,
    }
}

/// Decide whether `index`'s window could contain a record matching `query`.
///
/// Conservative by design: it may return `true` for a window that holds no real
/// match (the full query re-checks the decrypted records), but it never returns
/// `false` for a window that *does* contain one. That makes it safe as a
/// prefilter that skips fetching and decrypting irrelevant chunks.
pub fn might_match(index: &ServiceWindowIndex, query: &IndexQuery) -> bool {
    if let Some(service) = &query.service {
        if &index.service != service {
            return false;
        }
    }
    // No time overlap if the whole window is before `since` or after `until`.
    if let Some(since) = query.since {
        if index.max_timestamp < since {
            return false;
        }
    }
    if let Some(until) = query.until {
        if index.min_timestamp > until {
            return false;
        }
    }
    // The level/host sets are complete for the window, so absence means exclude.
    if let Some(level) = &query.level {
        if !index.levels.contains(level) {
            return false;
        }
    }
    if let Some(host) = &query.host {
        if !index.hosts.contains(host) {
            return false;
        }
    }
    if let Some(keyword) = &query.keyword {
        let mut tokens = BTreeSet::new();
        tokenize_into(keyword, &mut tokens);
        // If the keyword yields indexable tokens, every one must be present for
        // the window to possibly contain it.
        if !tokens.is_empty() && !tokens.iter().all(|t| index.keywords.contains(t)) {
            return false;
        }
    }
    true
}

/// Recursively tokenize the string values of a JSON value into `out`.
fn collect_tokens(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::String(s) => tokenize_into(s, out),
        Value::Array(items) => items.iter().for_each(|v| collect_tokens(v, out)),
        Value::Object(map) => map.values().for_each(|v| collect_tokens(v, out)),
        _ => {}
    }
}

/// Split `text` into lowercased alphanumeric word tokens (length 2..=64) and
/// insert them into `out`. Bounding the length keeps the index compact and drops
/// noise (single characters, opaque blobs) without losing real query terms.
fn tokenize_into(text: &str, out: &mut BTreeSet<String>) {
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        if raw.is_empty() {
            continue;
        }
        let token = raw.to_lowercase();
        if (2..=64).contains(&token.len()) {
            out.insert(token);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(secs, 0).unwrap()
    }

    fn record(level: &str, host: &str, msg: &str, secs: i64) -> LogRecord {
        LogRecord {
            raw: serde_json::json!({ "level": level, "host": host, "msg": msg }),
            timestamp: ts(secs),
            service: "api".to_string(),
            level: Some(level.to_string()),
            host: Some(host.to_string()),
            trace_id: None,
        }
    }

    fn sample_index() -> ServiceWindowIndex {
        let chunk = ChunkRef {
            service: "api".to_string(),
            window: "2026-06-29-15".to_string(),
            sequence: 0,
        };
        let records = vec![
            record("info", "host-1", "database connection established", 100),
            record("error", "host-2", "request timeout exceeded", 200),
        ];
        build_index(chunk, &records)
    }

    #[test]
    fn index_summarizes_the_window() {
        let index = sample_index();
        assert_eq!(index.service, "api");
        assert_eq!(index.window, "2026-06-29-15");
        assert_eq!(index.min_timestamp, ts(100));
        assert_eq!(index.max_timestamp, ts(200));
        assert!(index.levels.contains("info") && index.levels.contains("error"));
        assert!(index.hosts.contains("host-1") && index.hosts.contains("host-2"));
        assert!(index.keywords.contains("database"));
        assert!(index.keywords.contains("timeout"));
    }

    #[test]
    fn prefilter_includes_matching_windows() {
        let index = sample_index();
        assert!(might_match(&index, &IndexQuery::default()));
        assert!(might_match(
            &index,
            &IndexQuery {
                service: Some("api".into()),
                ..Default::default()
            }
        ));
        assert!(might_match(
            &index,
            &IndexQuery {
                level: Some("error".into()),
                ..Default::default()
            }
        ));
        assert!(might_match(
            &index,
            &IndexQuery {
                host: Some("host-1".into()),
                ..Default::default()
            }
        ));
        assert!(might_match(
            &index,
            &IndexQuery {
                keyword: Some("timeout".into()),
                ..Default::default()
            }
        ));
        // Time range that overlaps the window.
        assert!(might_match(
            &index,
            &IndexQuery {
                since: Some(ts(150)),
                until: Some(ts(250)),
                ..Default::default()
            }
        ));
    }

    #[test]
    fn prefilter_excludes_nonmatching_windows() {
        let index = sample_index();
        assert!(!might_match(
            &index,
            &IndexQuery {
                service: Some("web".into()),
                ..Default::default()
            }
        ));
        assert!(!might_match(
            &index,
            &IndexQuery {
                level: Some("warn".into()),
                ..Default::default()
            }
        ));
        assert!(!might_match(
            &index,
            &IndexQuery {
                host: Some("host-9".into()),
                ..Default::default()
            }
        ));
        assert!(!might_match(
            &index,
            &IndexQuery {
                keyword: Some("nonexistent".into()),
                ..Default::default()
            }
        ));
        // Window entirely before / after the requested range.
        assert!(!might_match(
            &index,
            &IndexQuery {
                since: Some(ts(300)),
                ..Default::default()
            }
        ));
        assert!(!might_match(
            &index,
            &IndexQuery {
                until: Some(ts(50)),
                ..Default::default()
            }
        ));
    }
}
