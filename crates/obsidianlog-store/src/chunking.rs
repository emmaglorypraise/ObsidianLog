//! Time-window chunking.
//!
//! A [`LogBatch`] is grouped into per-`(service, time_window)` buckets before it
//! is compressed, encrypted, and chained. Each record is routed by its service
//! and by the time window its timestamp falls into. Windows are a fixed number
//! of seconds (default one hour) and are labelled `YYYY-MM-DD-HH` from the
//! window's start instant, matching the on-storage chunk layout.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use obsidianlog_core::record::{LogBatch, LogRecord};

/// Default chunk time-window length, in seconds (1 hour).
pub const DEFAULT_WINDOW_SECS: u64 = 3600;

/// The records routed to one `(service, time_window)` chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct Bucket {
    /// Service the records belong to.
    pub service: String,
    /// Time-window label, `YYYY-MM-DD-HH`.
    pub window: String,
    /// Records in this window, in input order.
    pub records: Vec<LogRecord>,
}

/// The `YYYY-MM-DD-HH` label for the window of length `window_secs` that `ts`
/// falls into.
///
/// The label is formatted from the window's **start** instant, so every
/// timestamp within the same window maps to the same label (e.g. with a 1-hour
/// window, `15:00` and `15:59` both yield `…-15`).
pub fn window_label(ts: DateTime<Utc>, window_secs: u64) -> String {
    let secs = ts.timestamp();
    let window = window_secs.max(1) as i64;
    let start = secs - secs.rem_euclid(window);
    DateTime::<Utc>::from_timestamp(start, 0)
        .unwrap_or(ts)
        .format("%Y-%m-%d-%H")
        .to_string()
}

/// Group `batch` into per-`(service, time_window)` buckets using `window_secs`.
///
/// Buckets are returned sorted by `(service, window)`; records within a bucket
/// keep their input order.
pub fn chunk_batch(batch: &LogBatch, window_secs: u64) -> Vec<Bucket> {
    let mut groups: BTreeMap<(String, String), Vec<LogRecord>> = BTreeMap::new();
    for record in &batch.0 {
        let window = window_label(record.timestamp, window_secs);
        groups
            .entry((record.service.clone(), window))
            .or_default()
            .push(record.clone());
    }
    groups
        .into_iter()
        .map(|((service, window), records)| Bucket {
            service,
            window,
            records,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(service: &str, epoch_secs: i64) -> LogRecord {
        LogRecord {
            raw: serde_json::json!({ "t": epoch_secs }),
            timestamp: DateTime::<Utc>::from_timestamp(epoch_secs, 0).unwrap(),
            service: service.to_string(),
            level: None,
            host: None,
            trace_id: None,
        }
    }

    #[test]
    fn window_label_truncates_to_the_window_start() {
        // 1970-01-01 00:30:00 and 00:59:59 are both in the 00:00 hour window.
        assert_eq!(
            window_label(record("x", 1800).timestamp, 3600),
            "1970-01-01-00"
        );
        assert_eq!(
            window_label(record("x", 3599).timestamp, 3600),
            "1970-01-01-00"
        );
        assert_eq!(
            window_label(record("x", 3600).timestamp, 3600),
            "1970-01-01-01"
        );
    }

    #[test]
    fn groups_records_by_service_and_window() {
        let batch = LogBatch(vec![
            record("api", 0),    // api, 00 window
            record("api", 1800), // api, 00 window (same hour)
            record("api", 3600), // api, 01 window (next hour)
            record("web", 100),  // web, 00 window
        ]);

        let buckets = chunk_batch(&batch, DEFAULT_WINDOW_SECS);

        // Sorted by (service, window): api/00, api/01, web/00.
        assert_eq!(buckets.len(), 3);

        assert_eq!(buckets[0].service, "api");
        assert_eq!(buckets[0].window, "1970-01-01-00");
        assert_eq!(buckets[0].records.len(), 2);

        assert_eq!(buckets[1].service, "api");
        assert_eq!(buckets[1].window, "1970-01-01-01");
        assert_eq!(buckets[1].records.len(), 1);

        assert_eq!(buckets[2].service, "web");
        assert_eq!(buckets[2].window, "1970-01-01-00");
        assert_eq!(buckets[2].records.len(), 1);
    }

    #[test]
    fn configurable_window_size() {
        // A 6-hour window puts 00:00 and 05:59 together, 06:00 in the next.
        let six_hours = 6 * 3600;
        assert_eq!(
            window_label(record("x", 0).timestamp, six_hours),
            "1970-01-01-00"
        );
        assert_eq!(
            window_label(record("x", 5 * 3600).timestamp, six_hours),
            "1970-01-01-00"
        );
        assert_eq!(
            window_label(record("x", 6 * 3600).timestamp, six_hours),
            "1970-01-01-06"
        );
    }
}
