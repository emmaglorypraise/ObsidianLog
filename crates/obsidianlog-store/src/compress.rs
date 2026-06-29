//! zstd compression of log batches.
//!
//! Log text is highly compressible; the architecture targets large size
//! reductions. Backed by the `zstd` crate. Failures surface as
//! [`obsidianlog_core::Error::Io`] (zstd reports errors as `std::io::Error`).

use crate::error::Result;

/// Default zstd level for log data.
///
/// Level 3 is zstd's own default — a strong ratio/speed balance that compresses
/// repetitive log text heavily while staying cheap enough for the ingest path.
pub const DEFAULT_LEVEL: i32 = 3;

/// Compress `data` with zstd at the given `level`.
pub fn compress(data: &[u8], level: i32) -> Result<Vec<u8>> {
    Ok(zstd::encode_all(data, level)?)
}

/// Decompress a zstd stream produced by [`compress`].
pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    Ok(zstd::decode_all(data)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_arbitrary_bytes() {
        // A deterministic, not-very-compressible byte pattern.
        let data: Vec<u8> = (0..2000u32)
            .map(|i| (i.wrapping_mul(31) ^ 0x9E) as u8)
            .collect();
        let compressed = compress(&data, DEFAULT_LEVEL).expect("compress");
        let restored = decompress(&compressed).expect("decompress");
        assert_eq!(restored, data);
    }

    #[test]
    fn round_trips_empty_input() {
        let compressed = compress(&[], DEFAULT_LEVEL).expect("compress");
        assert_eq!(
            decompress(&compressed).expect("decompress"),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn compresses_realistic_json_logs_well() {
        // A realistic, multi-line JSON log sample (repetitive, as real logs are).
        let mut sample = String::new();
        for i in 0..200 {
            sample.push_str(&format!(
                "{{\"timestamp\":\"2026-06-29T15:00:{:02}Z\",\"service\":\"api\",\
                 \"level\":\"info\",\"host\":\"host-1\",\"trace_id\":\"abc123\",\
                 \"msg\":\"request handled\",\"path\":\"/v1/resource\",\"status\":200,\
                 \"latency_ms\":{}}}\n",
                i % 60,
                i
            ));
        }
        let data = sample.as_bytes();
        let compressed = compress(data, DEFAULT_LEVEL).expect("compress");

        // Sanity check (not a hard threshold): realistic logs compress heavily.
        assert!(
            compressed.len() * 4 < data.len(),
            "expected a high compression ratio: {} bytes -> {} bytes",
            data.len(),
            compressed.len()
        );
        assert_eq!(decompress(&compressed).expect("decompress"), data);
    }
}
