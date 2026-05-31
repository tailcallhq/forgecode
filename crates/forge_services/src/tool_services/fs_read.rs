use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use forge_app::{
    Content, EnvironmentInfra, FileInfoInfra, FileReaderInfra as InfraFsReadService, FsReadService,
    ReadOutput, compute_hash,
};
use forge_domain::{Document, FileInfo, Image};

use crate::range::resolve_range;
use crate::utils::assert_absolute_path;

/// Truncates a line to the maximum length if it exceeds the limit
fn truncate_line(line: &str, max_length: usize) -> String {
    if line.len() > max_length {
        // Use char indices to avoid panicking on unicode boundaries
        let truncated = line
            .char_indices()
            .take_while(|(idx, _)| *idx < max_length)
            .map(|(_, ch)| ch)
            .collect::<String>();
        format!(
            "{}... [truncated, line exceeds {} chars]",
            truncated, max_length
        )
    } else {
        line.to_string()
    }
}

/// Detects the MIME type of a file based on extension and content
fn detect_mime_type(path: &Path, content: &[u8]) -> String {
    // Try infer crate first (checks magic numbers)
    if let Some(file_type) = infer::get(content) {
        return file_type.mime_type().to_string();
    }

    // Fallback to extension-based detection
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| match ext.to_lowercase().as_str() {
            "txt" | "md" | "rs" | "toml" | "yaml" | "yml" | "json" | "js" | "ts" | "py" | "sh" => {
                "text/plain"
            }
            "ipynb" => "application/json",
            "pdf" => "application/pdf",
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "bmp" => "image/bmp",
            _ => "text/plain", // Default to text
        })
        .unwrap_or("text/plain")
        .to_string()
}

/// Checks if a MIME type represents visual content (images or PDFs)
fn is_visual_content(mime_type: &str) -> bool {
    is_image_content(mime_type) || is_document_content(mime_type)
}

/// Checks if a MIME type represents an image
fn is_image_content(mime_type: &str) -> bool {
    mime_type.starts_with("image/")
}

fn is_supported_image_content(mime_type: &str) -> bool {
    matches!(
        mime_type,
        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
    )
}

fn supported_image_formats() -> &'static str {
    "JPEG, PNG, GIF, WebP"
}

fn unsupported_image_format_error(path: &Path, mime_type: &str) -> anyhow::Error {
    let format = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_uppercase())
        .unwrap_or_else(|| mime_type.to_string());

    anyhow::anyhow!(
        "Unsupported image format: {format}. Supported formats: {}. Convert the file to a supported format, then try read again.",
        supported_image_formats()
    )
}

/// Checks if a MIME type represents a document (e.g., PDF)
fn is_document_content(mime_type: &str) -> bool {
    mime_type == "application/pdf"
}

/// Validates that file size does not exceed the maximum allowed file size.
///
/// # Arguments
/// * `infra` - The infrastructure instance providing file metadata services
/// * `path` - The file path to check
/// * `max_file_size` - Maximum allowed file size in bytes
///
/// # Returns
/// * `Ok(())` if file size is within limits
/// * `Err(anyhow::Error)` if file exceeds max_file_size
pub async fn assert_file_size<F: FileInfoInfra>(
    infra: &F,
    path: &Path,
    max_file_size: u64,
) -> anyhow::Result<()> {
    let file_size = infra.file_size(path).await?;
    if file_size > max_file_size {
        return Err(anyhow::anyhow!(
            "File size ({file_size} bytes) exceeds the maximum allowed size of {max_file_size} bytes"
        ));
    }
    Ok(())
}

/// Reads file contents from the specified absolute path. Ideal for analyzing
/// code, configuration files, documentation, or textual data. Returns the
/// content as a string. For files larger than 2,000 lines, the tool
/// automatically returns only the first 2,000 lines. Start with this default
/// chunk, then use targeted follow-up ranges when you need more context from a
/// specific part of the file. If needed, specify a range with the start_line
/// and end_line parameters, ensuring the total range does not exceed 2,000
/// lines.
/// Specifying a range exceeding this limit will result in an error. Binary
/// files are automatically detected and rejected.
pub struct ForgeFsRead<F> {
    infra: Arc<F>,
}

impl<F> ForgeFsRead<F> {
    pub fn new(infra: Arc<F>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<F: FileInfoInfra + EnvironmentInfra<Config = forge_config::ForgeConfig> + InfraFsReadService>
    FsReadService for ForgeFsRead<F>
{
    async fn read(
        &self,
        path: String,
        start_line: Option<u64>,
        end_line: Option<u64>,
    ) -> anyhow::Result<ReadOutput> {
        let path = Path::new(&path);
        assert_absolute_path(path)?;

        let config = self.infra.get_config()?;

        // Validate with the larger limit initially since we don't know file type yet
        let initial_size_limit = config.max_file_size_bytes.max(config.max_image_size_bytes);
        assert_file_size(&*self.infra, path, initial_size_limit).await?;

        // Read file content to detect MIME type
        let raw_content = self
            .infra
            .read(path)
            .await
            .with_context(|| format!("Failed to read file from {}", path.display()))?;

        // Detect MIME type
        let mime_type = detect_mime_type(path, &raw_content);

        // Handle visual content (PDFs and images)
        if is_visual_content(&mime_type) {
            if is_image_content(&mime_type) && !is_supported_image_content(&mime_type) {
                return Err(unsupported_image_format_error(path, &mime_type));
            }

            // Validate against image-specific size limit (may be different from
            // max_file_size)
            assert_file_size(&*self.infra, path, config.max_image_size_bytes).await.with_context(|| {
                if is_document_content(&mime_type) {
                    "PDF exceeds size limit. Use a smaller PDF or increase FORGE_MAX_IMAGE_SIZE."
                } else {
                    "Image exceeds size limit. Compress the image or increase FORGE_MAX_IMAGE_SIZE."
                }
            })?;

            if is_document_content(&mime_type) {
                // Return as Document for PDFs
                let filename = path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .map(|f| f.to_string());
                let document = Document::new_bytes(raw_content, mime_type.clone());
                let document = if let Some(name) = filename {
                    document.with_filename(name)
                } else {
                    document
                };
                let hash = compute_hash(document.base64_data());

                return Ok(ReadOutput {
                    content: Content::document(document),
                    info: FileInfo::new(0, 0, 0, hash),
                });
            }

            // Return as Image for images
            let image = Image::new_bytes(raw_content, mime_type.clone());
            let hash = compute_hash(image.url());

            return Ok(ReadOutput {
                content: Content::image(image),
                info: FileInfo::new(0, 0, 0, hash),
            });
        }

        // Handle text content (including Jupyter notebooks)
        // File size already validated above

        let (start_line, end_line) = resolve_range(start_line, end_line, config.max_read_lines);

        // Convert bytes to UTF-8 string
        let full_content = String::from_utf8(raw_content)
            .with_context(|| format!("Failed to read file as UTF-8 from {}", path.display()))?;

        let hash = compute_hash(&full_content);

        // Now extract the requested range from the content we already have
        let lines: Vec<&str> = full_content.lines().collect();
        let total_lines = lines.len() as u64;

        // Convert to 0-based indexing and clamp to valid range
        let start_pos = start_line
            .saturating_sub(1)
            .min(total_lines.saturating_sub(1));
        let end_pos = end_line
            .saturating_sub(1)
            .min(total_lines.saturating_sub(1));

        // Extract requested lines
        let content = if start_pos == 0 && end_pos >= total_lines.saturating_sub(1) {
            // Return full content with line truncation
            lines
                .iter()
                .map(|line| truncate_line(line, config.max_line_chars))
                .collect::<Vec<_>>()
                .join("\n")
        } else if total_lines == 0 {
            String::new()
        } else {
            // Return range with line truncation
            lines
                .get(start_pos as usize..=end_pos as usize)
                .map(|slice| {
                    slice
                        .iter()
                        .map(|line| truncate_line(line, config.max_line_chars))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default()
        };

        let file_info = FileInfo::new(start_line, end_line, total_lines, hash);

        Ok(ReadOutput { content: Content::file(content), info: file_info })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use forge_app::FsReadService;
    use pretty_assertions::assert_eq;
    use tempfile::NamedTempFile;
    use tokio::fs;

    use super::*;
    use crate::attachment::tests::{MockCompositeService, MockFileService};

    // Helper to create a temporary file with specific content size
    async fn create_test_file_with_size(size: usize) -> anyhow::Result<NamedTempFile> {
        let file = NamedTempFile::new()?;
        let content = "x".repeat(size);
        fs::write(file.path(), content).await?;
        Ok(file)
    }

    #[tokio::test]
    async fn test_assert_file_size_within_limit() {
        let fixture = create_test_file_with_size(13).await.unwrap();
        let infra = MockFileService::new();
        // Add the file to the mock infrastructure
        infra.add_file(fixture.path().to_path_buf(), "x".repeat(13));
        let actual = assert_file_size(&infra, fixture.path(), 20u64).await;
        assert!(actual.is_ok());
    }

    #[tokio::test]
    async fn test_assert_file_size_exactly_at_limit() {
        let fixture = create_test_file_with_size(6).await.unwrap();
        let infra = MockFileService::new();
        infra.add_file(fixture.path().to_path_buf(), "x".repeat(6));
        let actual = assert_file_size(&infra, fixture.path(), 6u64).await;
        assert!(actual.is_ok());
    }

    #[tokio::test]
    async fn test_assert_file_size_exceeds_limit() {
        let fixture = create_test_file_with_size(45).await.unwrap();
        let infra = MockFileService::new();
        infra.add_file(fixture.path().to_path_buf(), "x".repeat(45));
        let actual = assert_file_size(&infra, fixture.path(), 10u64).await;
        assert!(actual.is_err());
    }

    #[tokio::test]
    async fn test_assert_file_size_empty_content() {
        let fixture = create_test_file_with_size(0).await.unwrap();
        let infra = MockFileService::new();
        infra.add_file(fixture.path().to_path_buf(), "".to_string());
        let actual = assert_file_size(&infra, fixture.path(), 100u64).await;
        assert!(actual.is_ok());
    }

    #[tokio::test]
    async fn test_assert_file_size_zero_limit() {
        let fixture = create_test_file_with_size(1).await.unwrap();
        let infra = MockFileService::new();
        infra.add_file(fixture.path().to_path_buf(), "x".to_string());
        let actual = assert_file_size(&infra, fixture.path(), 0u64).await;
        assert!(actual.is_err());
    }

    #[tokio::test]
    async fn test_assert_file_size_large_content() {
        let fixture = create_test_file_with_size(1000).await.unwrap();
        let infra = MockFileService::new();
        infra.add_file(fixture.path().to_path_buf(), "x".repeat(1000));
        let actual = assert_file_size(&infra, fixture.path(), 999u64).await;
        assert!(actual.is_err());
    }

    #[tokio::test]
    async fn test_assert_file_size_large_content_within_limit() {
        let fixture = create_test_file_with_size(1000).await.unwrap();
        let infra = MockFileService::new();
        infra.add_file(fixture.path().to_path_buf(), "x".repeat(1000));
        let actual = assert_file_size(&infra, fixture.path(), 1000u64).await;
        assert!(actual.is_ok());
    }

    #[tokio::test]
    async fn test_assert_file_size_unicode_content() {
        let file = NamedTempFile::new().unwrap();
        fs::write(file.path(), "🚀🚀🚀").await.unwrap(); // Each emoji is 4 bytes in UTF-8 = 12 bytes total
        let infra = MockFileService::new();
        infra.add_file(file.path().to_path_buf(), "🚀🚀🚀".to_string());
        let actual = assert_file_size(&infra, file.path(), 12u64).await;
        assert!(actual.is_ok());
    }

    #[tokio::test]
    async fn test_assert_file_size_unicode_content_exceeds() {
        let file = NamedTempFile::new().unwrap();
        fs::write(file.path(), "🚀🚀🚀🚀").await.unwrap(); // 4 emojis = 16 bytes, exceeds 12 byte limit
        let infra = MockFileService::new();
        infra.add_file(file.path().to_path_buf(), "🚀🚀🚀🚀".to_string());
        let actual = assert_file_size(&infra, file.path(), 12u64).await;
        assert!(actual.is_err());
    }

    #[tokio::test]
    async fn test_assert_file_size_error_message() {
        let file = NamedTempFile::new().unwrap();
        fs::write(file.path(), "too long content").await.unwrap(); // 16 bytes
        let infra = MockFileService::new();
        infra.add_file(file.path().to_path_buf(), "too long content".to_string());
        let actual = assert_file_size(&infra, file.path(), 5u64).await;
        let expected = "File size (16 bytes) exceeds the maximum allowed size of 5 bytes";
        assert!(actual.is_err());
        assert_eq!(actual.unwrap_err().to_string(), expected);
    }

    #[test]
    fn test_detect_mime_type_for_text_files() {
        let path = Path::new("test.txt");
        let content = b"Hello, world!";
        let actual = detect_mime_type(path, content);
        assert_eq!(actual, "text/plain");
    }

    #[test]
    fn test_detect_mime_type_for_ipynb() {
        let path = Path::new("notebook.ipynb");
        let content = b"{\"cells\": []}";
        let actual = detect_mime_type(path, content);
        assert_eq!(actual, "application/json");
    }

    #[test]
    fn test_detect_mime_type_for_png() {
        let path = Path::new("image.png");
        // PNG magic number
        let content = b"\x89PNG\r\n\x1a\n";
        let actual = detect_mime_type(path, content);
        assert_eq!(actual, "image/png");
    }

    #[test]
    fn test_detect_mime_type_for_pdf_with_magic() {
        let path = Path::new("document.pdf");
        // PDF magic number
        let content = b"%PDF-1.4";
        let actual = detect_mime_type(path, content);
        assert_eq!(actual, "application/pdf");
    }

    #[test]
    fn test_detect_mime_type_for_jpeg() {
        let path = Path::new("photo.jpg");
        // JPEG magic number
        let content = b"\xFF\xD8\xFF";
        let actual = detect_mime_type(path, content);
        assert_eq!(actual, "image/jpeg");
    }

    #[test]
    fn test_detect_mime_type_for_bmp_extension() {
        let path = Path::new("thumbnail.bmp");
        let content = b"not-a-real-bmp";
        let actual = detect_mime_type(path, content);
        assert_eq!(actual, "image/bmp");
    }

    #[test]
    fn test_is_visual_content_for_images() {
        assert!(is_visual_content("image/png"));
        assert!(is_visual_content("image/jpeg"));
        assert!(is_visual_content("image/gif"));
        assert!(is_visual_content("image/webp"));
    }

    #[test]
    fn test_is_supported_image_content() {
        assert!(is_supported_image_content("image/png"));
        assert!(is_supported_image_content("image/jpeg"));
        assert!(!is_supported_image_content("image/bmp"));
    }

    #[test]
    fn test_is_visual_content_for_pdf() {
        assert!(is_visual_content("application/pdf"));
    }

    #[test]
    fn test_is_visual_content_for_text() {
        assert!(!is_visual_content("text/plain"));
        assert!(!is_visual_content("application/json"));
        assert!(!is_visual_content("text/html"));
    }

    #[tokio::test]
    async fn test_read_rejects_unsupported_image_formats() {
        let infra = Arc::new(MockCompositeService::new());
        infra.add_bytes(
            Path::new("/tmp/thumbnail.bmp").to_path_buf(),
            b"not-a-real-bmp".to_vec(),
        );

        let service = ForgeFsRead::new(infra);
        let error = <ForgeFsRead<MockCompositeService> as FsReadService>::read(
            &service,
            "/tmp/thumbnail.bmp".to_string(),
            None,
            None,
        )
        .await
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Unsupported image format: BMP. Supported formats: JPEG, PNG, GIF, WebP. Convert the file to a supported format, then try read again."
        );
    }

    #[test]
    fn test_truncate_line_short_line() {
        let line = "short line";
        let actual = truncate_line(line, 100);
        assert_eq!(actual, "short line");
    }

    #[test]
    fn test_truncate_line_exact_length() {
        let line = "exactly 17 chars!";
        assert_eq!(line.len(), 17);
        let actual = truncate_line(line, 17);
        assert_eq!(actual, "exactly 17 chars!");
    }

    #[test]
    fn test_truncate_line_long_line() {
        let line = "this is a very long line that exceeds the maximum length";
        let actual = truncate_line(line, 20);
        assert_eq!(actual.len(), 58); // 20 chars + "... [truncated, line exceeds 20 chars]"
        assert!(actual.starts_with("this is a very long"));
        assert!(actual.contains("[truncated"));
        assert!(!actual.contains("exceeds the maximum length"));
    }

    #[test]
    fn test_truncate_line_empty() {
        let line = "";
        let actual = truncate_line(line, 100);
        assert_eq!(actual, "");
    }

    #[test]
    fn test_truncate_line_unicode() {
        let line = "🚀🚀🚀🚀🚀"; // Each emoji is 4 chars, total 20
        let actual = truncate_line(line, 12);
        // Should truncate at byte boundary, not character boundary
        println!("{}", actual);
        assert_eq!(actual.len(), 50); // 12 bytes + truncation message
        assert!(actual.contains("truncated"));
    }
}
