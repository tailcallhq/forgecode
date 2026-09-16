use anyhow::Result;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

/// Detected encoding types based on BOM analysis
#[derive(Debug, Clone, PartialEq, Eq)]
enum Encoding {
    Utf8WithBom,
    Utf16BE,
    Utf16LE,
}

impl Encoding {
    /// Detects BOM (Byte Order Mark) patterns
    pub fn detect(buffer: &[u8], bytes_read: usize) -> Option<Self> {
        match (buffer.first(), buffer.get(1), buffer.get(2)) {
            (Some(&0xEF), Some(&0xBB), Some(&0xBF)) if bytes_read >= 3 => {
                Some(Encoding::Utf8WithBom)
            }
            (Some(&0xFE), Some(&0xFF), _) if bytes_read >= 2 => Some(Encoding::Utf16BE),
            (Some(&0xFF), Some(&0xFE), _) if bytes_read >= 2 => Some(Encoding::Utf16LE),
            _ => None,
        }
    }
}

/// Detects if a file is binary by analyzing its content
pub async fn is_binary<P: AsRef<std::path::Path>>(path: P) -> Result<bool> {
    use anyhow::Context;
    let path_ref = path.as_ref();
    let mut file = File::open(path_ref)
        .await
        .with_context(|| format!("Failed to open file {}", path_ref.display()))?;

    let mut buffer = vec![0u8; 512];
    let bytes_read = file.read(&mut buffer).await?;
    buffer.truncate(bytes_read);

    Ok(is_binary_internal(&buffer, bytes_read))
}

/// Detects encoding and binary status from a buffer
fn is_binary_internal(buffer: &[u8], bytes_read: usize) -> bool {
    // Always first check for BOM to find out about encoding
    let encoding = Encoding::detect(buffer, bytes_read);

    // Detect 0 bytes to see if file is binary or UTF-16 LE/BE
    // unless we already know that this file has a UTF-16 encoding
    let mut seems_binary = false;
    if encoding != Some(Encoding::Utf16BE)
        && encoding != Some(Encoding::Utf16LE)
        && !buffer.is_empty()
    {
        let mut could_be_utf16le = true; // e.g. 0xAA 0x00
        let mut could_be_utf16be = true; // e.g. 0x00 0xAA
        let mut contains_zero_byte = false;

        // This is a simplified guess to detect UTF-16 BE or LE by just checking
        // if the first 512 bytes have the 0-byte at a specific
        // location. For UTF-16 LE this would be the odd byte index and
        // for UTF-16 BE the even one. Note: this can produce false
        // positives (a binary file that uses a 2-byte encoding of the
        // same format as UTF-16) and false negatives (a UTF-16 file
        // that is using 4 bytes to encode a character).
        const ZERO_BYTE_DETECTION_BUFFER_MAX_LEN: usize = 512;
        for (i, &byte) in buffer
            .iter()
            .enumerate()
            .take(bytes_read.min(ZERO_BYTE_DETECTION_BUFFER_MAX_LEN))
        {
            let is_endian = i % 2 == 1; // assume 2-byte sequences typical for UTF-16
            let is_zero_byte = byte == 0;

            if is_zero_byte {
                contains_zero_byte = true;
            }

            // UTF-16 LE: expect e.g. 0xAA 0x00
            if could_be_utf16le && (is_endian && !is_zero_byte || !is_endian && is_zero_byte) {
                could_be_utf16le = false;
            }

            // UTF-16 BE: expect e.g. 0x00 0xAA
            if could_be_utf16be && (is_endian && is_zero_byte || !is_endian && !is_zero_byte) {
                could_be_utf16be = false;
            }

            // Return if this is neither UTF16-LE nor UTF16-BE and thus treat as
            // binary
            if is_zero_byte && !could_be_utf16le && !could_be_utf16be {
                break;
            }
        }

        // Handle case of 0-byte included
        if contains_zero_byte && !could_be_utf16le && !could_be_utf16be {
            seems_binary = true;
        }
    }

    seems_binary
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tempfile::NamedTempFile;
    use tokio::fs;

    use super::*;

    async fn create_test_file_fixture(content: &[u8]) -> Result<NamedTempFile> {
        let file = NamedTempFile::new()?;
        fs::write(file.path(), content).await?;
        Ok(file)
    }

    #[tokio::test]
    async fn test_empty_file_is_binary() -> Result<()> {
        let fixture = create_test_file_fixture(&[]).await?;
        let actual = is_binary(fixture.path()).await?;
        let expected = false;
        assert_eq!(actual, expected);
        Ok(())
    }

    #[tokio::test]
    async fn test_plain_text_file_is_binary() -> Result<()> {
        let fixture = create_test_file_fixture(b"Hello, world!").await?;
        let actual = is_binary(fixture.path()).await?;
        let expected = false;
        assert_eq!(actual, expected);
        Ok(())
    }

    #[tokio::test]
    async fn test_binary_file_with_zero_bytes() -> Result<()> {
        let content = vec![
            0x48, 0x65, 0x6C, 0x6C, 0x6F, 0x00, 0x57, 0x6F, 0x72, 0x6C, 0x64,
        ];
        let fixture = create_test_file_fixture(&content).await?;
        let actual = is_binary(fixture.path()).await?;
        let expected = true;
        assert_eq!(actual, expected);
        Ok(())
    }

    #[tokio::test]
    async fn test_buffer_limit_512_bytes() -> Result<()> {
        // Create content larger than 512 bytes with zero byte at position 600
        let mut content = vec![0x48; 600]; // 'H' repeated 600 times
        content[599] = 0x00; // Zero byte beyond 512 byte limit

        let fixture = create_test_file_fixture(&content).await?;
        let actual = is_binary(fixture.path()).await?;

        // Should not detect as binary because zero byte is beyond 512-byte
        // limit
        let expected = false;
        assert_eq!(actual, expected);
        Ok(())
    }
}
