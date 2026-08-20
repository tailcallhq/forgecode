/// Transparent zstd compression and decompression codec
///
/// This codec provides lossless, reversible compression of context JSON blobs.
/// Matches the exact codec from forge_repo/src/codec/compression.rs.
/// Compression uses zstd level 3 (fast, ~4x on JSON).
use anyhow::Context;

/// Compress a string to zstd-compressed bytes (level 3)
///
/// # Arguments
/// * `s` - JSON string to compress
///
/// # Returns
/// Result with compressed bytes or error
pub fn compress(s: &str) -> anyhow::Result<Vec<u8>> {
    let bytes = s.as_bytes();
    zstd::encode_all(bytes, 3).context("Failed to compress context blob with zstd")
}

/// Decompress zstd-compressed bytes to string
///
/// # Arguments
/// * `b` - Compressed bytes (zstd format)
///
/// # Returns
/// Result with decompressed JSON string or error
pub fn decompress(b: &[u8]) -> anyhow::Result<String> {
    let decompressed = zstd::decode_all(b)
        .context("Failed to decompress context blob with zstd")?;

    String::from_utf8(decompressed)
        .context("Decompressed context blob is not valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_trip_small_json() {
        let json = r#"{"id":"conv-123","messages":[]}"#;
        let compressed = compress(json).expect("compress should not fail");
        let decompressed = decompress(&compressed).expect("decompress should not fail");
        assert_eq!(decompressed, json);
    }

    #[test]
    fn test_round_trip_large_json() {
        // Simulate large context blob with many messages
        let mut json = r#"{"id":"conv-large","messages":["#.to_string();
        for i in 0..1000 {
            json.push_str(&format!(
                r#"{{"role":"user","content":"message {}"}}"#,
                i
            ));
            if i < 999 {
                json.push(',');
            }
        }
        json.push_str("]}");

        let compressed = compress(&json).expect("compress should not fail");
        let decompressed = decompress(&compressed).expect("decompress should not fail");
        assert_eq!(decompressed, json);
        // Verify compression actually reduced size significantly
        assert!(
            compressed.len() < json.len() / 3,
            "compression ratio should be > 3x for this data"
        );
    }

    #[test]
    fn test_round_trip_empty_string() {
        let json = "";
        let compressed = compress(json).expect("compress should not fail");
        let decompressed = decompress(&compressed).expect("decompress should not fail");
        assert_eq!(decompressed, json);
    }

    #[test]
    fn test_round_trip_unicode() {
        let json = r#"{"content":"Hello 世界 🌍 مرحبا"}"#;
        let compressed = compress(json).expect("compress should not fail");
        let decompressed = decompress(&compressed).expect("decompress should not fail");
        assert_eq!(decompressed, json);
    }

    #[test]
    fn test_decompress_invalid_data() {
        let invalid_data = vec![0xFF, 0xFF, 0xFF];
        let result = decompress(&invalid_data);
        assert!(result.is_err(), "decompress should fail on invalid data");
    }

    #[test]
    fn test_compression_ratio() {
        // JSON with high redundancy compresses well
        let json = r#"{"data":["#.to_string()
            + &"[\"value\"],".repeat(100)
            + "]}";

        let compressed = compress(&json).expect("compress should not fail");
        let ratio = json.len() as f64 / compressed.len() as f64;
        assert!(
            ratio > 3.0,
            "compression ratio should be > 3x for redundant data, got {}",
            ratio
        );
    }
}
