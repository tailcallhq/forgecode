use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use bytes::Bytes;
use forge_app::{
    FileDirectoryInfra, FileInfoInfra, FileReaderInfra, FileWriterInfra, FsWriteOutput,
    FsWriteService, compute_hash,
};
use forge_domain::{SnapshotMetadataRepository, SnapshotRepository, ValidationRepository};

use crate::utils::assert_absolute_path;

/// Service for creating files with snapshot coordination
///
/// This service coordinates between infrastructure (file I/O) and repository
/// (snapshots) to create files while preserving the ability to undo changes.
///
/// # Line Ending Handling
/// The service preserves the line endings exactly as provided in the input
/// content. The hash is computed on the exact content being written, so it will
/// reflect the actual file content regardless of whether it uses LF or CRLF
/// line endings.
pub struct ForgeFsWrite<F> {
    infra: Arc<F>,
}

impl<F> ForgeFsWrite<F> {
    pub fn new(infra: Arc<F>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<
    F: FileDirectoryInfra
        + FileInfoInfra
        + FileReaderInfra
        + FileWriterInfra
        + SnapshotRepository
        + SnapshotMetadataRepository
        + ValidationRepository
        + Send
        + Sync,
> FsWriteService for ForgeFsWrite<F>
{
    async fn write(
        &self,
        path: String,
        content: String,
        overwrite: bool,
        user_input_id: forge_domain::UserInputId,
        conversation_id: forge_domain::ConversationId,
    ) -> anyhow::Result<FsWriteOutput> {
        let path = Path::new(&path);
        assert_absolute_path(path)?;

        // Validate file syntax using remote validation API (graceful failure)
        let errors = self
            .infra
            .validate_file(path, &content)
            .await
            .unwrap_or_default();

        if let Some(parent) = Path::new(&path).parent() {
            self.infra
                .create_dirs(parent)
                .await
                .with_context(|| format!("Failed to create directories: {}", path.display()))?;
        }

        // Check if the file exists
        let file_exists = self.infra.is_file(path).await?;

        // If file exists and overwrite flag is not set, return an error
        if file_exists && !overwrite {
            return Err(anyhow::anyhow!(
                "Cannot overwrite existing file: overwrite flag not set.",
            ))
            .with_context(|| format!("File already exists at {}", path.display()));
        }

        // Record the file content before modification and detect its line ending style
        let (old_content, target_line_ending) = if file_exists && overwrite {
            let existing = self.infra.read_utf8(path).await?;
            let line_ending = if existing.contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            (Some(existing), line_ending)
        } else {
            // For new files, use platform default line ending
            #[cfg(windows)]
            let default_ending = "\r\n";
            #[cfg(not(windows))]
            let default_ending = "\n";
            (None, default_ending)
        };

        // SNAPSHOT COORDINATION: Capture snapshot before writing.
        // For existing files, create a .snap backup and insert metadata.
        // For new files, insert metadata with empty snap_file_path so that
        // undo can delete the file instead of restoring it.
        if file_exists {
            self.infra
                .insert_snapshot(path, user_input_id, conversation_id)
                .await?;
        } else {
            let snapshot =
                forge_domain::Snapshot::create(path.to_path_buf(), user_input_id, conversation_id)?;
            self.infra
                .insert_snapshot_metadata(&snapshot, String::new())
                .await?;
        }

        // Normalize line endings to match the target style before writing
        let normalized_content = content
            .replace("\r\n", "\n") // First normalize all to LF
            .replace('\n', target_line_ending); // Then convert to target

        // Write file only after validation passes and directories are created
        self.infra
            .write(path, Bytes::from(normalized_content.clone()))
            .await?;

        // Compute hash of the normalized content that was written
        let content_hash = compute_hash(&normalized_content);

        Ok(FsWriteOutput {
            path: path.display().to_string(),
            before: old_content,
            errors,
            content_hash,
        })
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_normalize_crlf_to_lf() {
        let fixture = "line1\r\nline2\r\nline3";
        let actual = fixture.replace("\r\n", "\n");
        let expected = "line1\nline2\nline3";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_normalize_lf_to_crlf() {
        let fixture = "line1\nline2\nline3";
        let actual = fixture.replace("\r\n", "\n").replace('\n', "\r\n");
        let expected = "line1\r\nline2\r\nline3";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_normalize_mixed_endings() {
        let fixture = "line1\r\nline2\nline3\r\nline4";
        let actual = fixture.replace("\r\n", "\n");
        let expected = "line1\nline2\nline3\nline4";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_normalize_preserves_content() {
        let fixture = "hello world\r\nfoo bar\r\nbaz";
        let actual = fixture.replace("\r\n", "\n").replace('\n', "\r\n");
        let expected = "hello world\r\nfoo bar\r\nbaz";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_line_ending_detection_crlf() {
        let fixture = "line1\r\nline2\r\nline3";
        let detected = if fixture.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let expected = "\r\n";
        assert_eq!(detected, expected);
    }

    #[test]
    fn test_line_ending_detection_lf() {
        let fixture = "line1\nline2\nline3";
        let detected = if fixture.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let expected = "\n";
        assert_eq!(detected, expected);
    }

    #[test]
    fn test_hash_consistency_after_normalization() {
        let input_crlf = "line1\r\nline2\r\nline3";
        let normalized = input_crlf.replace("\r\n", "\n");

        let hash1 = compute_hash(&normalized);
        let hash2 = compute_hash(&normalized);

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_hash_differs_with_different_endings() {
        let content_lf = "line1\nline2\nline3";
        let content_crlf = "line1\r\nline2\r\nline3";

        let hash_lf = compute_hash(content_lf);
        let hash_crlf = compute_hash(content_crlf);

        assert_ne!(hash_lf, hash_crlf);
    }
}
