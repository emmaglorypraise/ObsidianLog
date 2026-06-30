//! Parsing raw Vector log events into [`LogRecord`]s.
//!
//! Vector posts JSON log events. This module extracts the fields ObsidianLog
//! routes and indexes on — timestamp, service, level, host, trace_id — and is
//! deliberately **tolerant**: missing fields become `None`, an unparseable
//! timestamp falls back to ingest time (flagged), and malformed input never
//! panics. The original event is preserved verbatim in [`LogRecord::raw`].

use chrono::{DateTime, Utc};
use serde_json::Value;

use obsidianlog_core::record::LogRecord;

/// Service assigned to a record that carries no `service` field.
pub const UNKNOWN_SERVICE: &str = "unknown";

/// A parsed record plus whether its timestamp had to be estimated.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedRecord {
    /// The extracted record.
    pub record: LogRecord,
    /// `true` if the input had no parseable timestamp and ingest time was used.
    pub timestamp_estimated: bool,
}

/// Parse a single raw JSON line into a [`ParsedRecord`].
///
/// Never fails: input that isn't valid JSON is still archived, with the raw text
/// preserved and ingest time used as the (estimated) timestamp.
pub fn parse_line(line: &str, ingest_time: DateTime<Utc>) -> ParsedRecord {
    match serde_json::from_str::<Value>(line) {
        Ok(value) => parse_value(value, ingest_time),
        Err(_) => ParsedRecord {
            record: LogRecord {
                raw: Value::String(line.to_string()),
                timestamp: ingest_time,
                service: UNKNOWN_SERVICE.to_string(),
                level: None,
                host: None,
                trace_id: None,
            },
            timestamp_estimated: true,
        },
    }
}

/// Parse an already-decoded JSON event into a [`ParsedRecord`].
pub fn parse_value(value: Value, ingest_time: DateTime<Utc>) -> ParsedRecord {
    let service = str_field(&value, "service").unwrap_or_else(|| UNKNOWN_SERVICE.to_string());
    let level = str_field(&value, "level");
    let host = str_field(&value, "host");
    let trace_id = str_field(&value, "trace_id");
    let (timestamp, timestamp_estimated) = parse_timestamp(&value, ingest_time);

    ParsedRecord {
        record: LogRecord {
            raw: value,
            timestamp,
            service,
            level,
            host,
            trace_id,
        },
        timestamp_estimated,
    }
}

/// Extract a string field, returning `None` if absent or not a string.
fn str_field(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

/// Parse the event timestamp, falling back to `ingest_time` (flagged) when it is
/// absent or unparseable. Accepts RFC 3339 strings and integer epoch seconds.
fn parse_timestamp(value: &Value, ingest_time: DateTime<Utc>) -> (DateTime<Utc>, bool) {
    let field = value
        .get("timestamp")
        .or_else(|| value.get("@timestamp"))
        .or_else(|| value.get("time"));

    if let Some(ts) = field {
        if let Some(text) = ts.as_str() {
            if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
                return (dt.with_timezone(&Utc), false);
            }
        } else if let Some(secs) = ts.as_i64() {
            if let Some(dt) = DateTime::<Utc>::from_timestamp(secs, 0) {
                return (dt, false);
            }
        }
    }
    (ingest_time, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ingest() -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_000_000, 0).unwrap()
    }

    fn rfc3339(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn parses_a_well_formed_event() {
        let value = serde_json::json!({
            "timestamp": "2026-06-29T15:04:05Z",
            "service": "api",
            "level": "error",
            "host": "host-1",
            "trace_id": "abc123",
            "msg": "boom"
        });
        let parsed = parse_value(value, ingest());

        assert!(!parsed.timestamp_estimated);
        assert_eq!(parsed.record.timestamp, rfc3339("2026-06-29T15:04:05Z"));
        assert_eq!(parsed.record.service, "api");
        assert_eq!(parsed.record.level.as_deref(), Some("error"));
        assert_eq!(parsed.record.host.as_deref(), Some("host-1"));
        assert_eq!(parsed.record.trace_id.as_deref(), Some("abc123"));
    }

    #[test]
    fn missing_optional_fields_become_none() {
        let parsed = parse_value(serde_json::json!({ "service": "api" }), ingest());
        assert!(parsed.record.level.is_none());
        assert!(parsed.record.host.is_none());
        assert!(parsed.record.trace_id.is_none());
        // No timestamp present → estimated, using ingest time.
        assert!(parsed.timestamp_estimated);
        assert_eq!(parsed.record.timestamp, ingest());
    }

    #[test]
    fn missing_service_defaults_to_unknown() {
        let parsed = parse_value(serde_json::json!({ "level": "info" }), ingest());
        assert_eq!(parsed.record.service, UNKNOWN_SERVICE);
        assert_eq!(parsed.record.level.as_deref(), Some("info"));
    }

    #[test]
    fn unparseable_timestamp_falls_back_to_ingest_time() {
        let parsed = parse_value(
            serde_json::json!({ "timestamp": "not-a-date", "service": "api" }),
            ingest(),
        );
        assert!(parsed.timestamp_estimated);
        assert_eq!(parsed.record.timestamp, ingest());
    }

    #[test]
    fn integer_epoch_timestamp_is_accepted() {
        let parsed = parse_value(
            serde_json::json!({ "timestamp": 1_700_000_000_i64, "service": "api" }),
            ingest(),
        );
        assert!(!parsed.timestamp_estimated);
        assert_eq!(
            parsed.record.timestamp,
            DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap()
        );
    }

    #[test]
    fn malformed_json_never_panics() {
        let parsed = parse_line("{ this is not valid json", ingest());
        assert!(parsed.timestamp_estimated);
        assert_eq!(parsed.record.service, UNKNOWN_SERVICE);
        assert_eq!(parsed.record.timestamp, ingest());
    }

    #[test]
    fn non_object_json_is_tolerated() {
        // A bare JSON array has no fields; everything defaults, nothing panics.
        let parsed = parse_value(serde_json::json!([1, 2, 3]), ingest());
        assert_eq!(parsed.record.service, UNKNOWN_SERVICE);
        assert!(parsed.timestamp_estimated);
    }
}
