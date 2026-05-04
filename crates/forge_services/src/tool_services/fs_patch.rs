use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;
use forge_app::domain::PatchOperation;
use forge_app::{FileWriterInfra, FsPatchService, PatchOutput, compute_hash};
use forge_domain::{
    FuzzySearchRepository, SearchMatch, SnapshotRepository, TextPatchBlock, TextPatchRepository,
    ValidationRepository,
};
use thiserror::Error;
use tokio::fs;

use crate::utils::assert_absolute_path;

/// A match found in the source text. Represents a range in the source text that
/// can be used for extraction or replacement operations. Stores the position
/// and length to allow efficient substring operations.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
struct Range {
    /// Starting position of the match in source text
    start: usize,
    /// Length of the matched text
    length: usize,
}

impl Range {
    /// Create a new match from a start position and length
    fn new(start: usize, length: usize) -> Self {
        Self { start, length }
    }

    /// Get the end position (exclusive) of this match
    fn end(&self) -> usize {
        self.start + self.length
    }

    /// Try to find an exact match in the source text
    fn find_exact(source: &str, search: &str) -> Option<Self> {
        source
            .find(search)
            .map(|start| Self::new(start, search.len()))
    }

    /// Detect the line ending used in the source (CRLF or LF)
    fn detect_line_ending(source: &str) -> &'static str {
        if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        }
    }

    /// Normalize line endings in a search string to match the source
    fn normalize_search_line_endings(source: &str, search: &str) -> String {
        let line_ending = Self::detect_line_ending(source);
        if line_ending == "\r\n" {
            search.replace("\r\n", "\n").replace("\n", "\r\n")
        } else {
            search.replace("\r\n", "\n")
        }
    }

    /// Create a range from a fuzzy search match
    #[allow(dead_code)]
    fn from_search_match(source: &str, search_match: &SearchMatch) -> Self {
        let lines: Vec<&str> = source.lines().collect();

        // Handle empty source
        if lines.is_empty() {
            return Self::new(0, 0);
        }

        // SearchMatch uses 0-based inclusive line numbers
        // Convert to 0-based array indices
        let start_idx = (search_match.start_line as usize).min(lines.len());
        // end_line is 0-based inclusive, convert to 0-based exclusive for slicing
        // Add 1 to make it exclusive: line 0 to line 0 means [0..1], one line
        let end_idx = ((search_match.end_line as usize) + 1).min(lines.len());

        // Find the byte position of the start line.
        // Split on '\n' so each segment retains its '\r' (if any), giving the
        // correct per-line byte length regardless of mixed line endings.
        let start_pos = source
            .split('\n')
            .take(start_idx)
            .map(|l| l.len() + 1)
            .sum::<usize>()
            .min(source.len());

        // Calculate the length
        let length = if start_idx == end_idx {
            // Single line match: just the line content, no trailing newline
            if start_idx >= lines.len() {
                0 // Out of bounds match
            } else {
                lines.get(start_idx).map_or(0, |l| l.len())
            }
        } else {
            // Multi-line match: include newlines between lines but NOT after the last line
            // Sum lengths of lines from start_idx to end_idx (exclusive)
            let content_len: usize = if start_idx >= lines.len() || end_idx > lines.len() {
                0 // Out of bounds match
            } else {
                lines
                    .get(start_idx..end_idx)
                    .map_or(0, |slice| slice.iter().map(|l| l.len()).sum())
            };
            let newlines_between = end_idx - start_idx - 1;
            // Count actual newline bytes (\r\n = 2, \n = 1) to handle mixed endings
            let newline_bytes: usize = source
                .split('\n')
                .skip(start_idx)
                .take(newlines_between)
                .map(|l| if l.ends_with('\r') { 2 } else { 1 })
                .sum();
            content_len + newline_bytes
        };

        Self::new(start_pos, length)
    }

    // Fuzzy matching removed - we only use exact matching
}

impl From<Range> for std::ops::Range<usize> {
    fn from(m: Range) -> Self {
        m.start..m.end()
    }
}

// MatchSequence struct and implementation removed - we only use exact matching

#[derive(Debug, Error)]
enum Error {
    #[error("Failed to read/write file: {0}")]
    FileOperation(#[from] std::io::Error),
    #[error(
        "Could not find match for search text: '{0}'. File may have changed externally, consider reading the file again."
    )]
    NoMatch(String),
    #[error("Could not find swap target text: {0}")]
    NoSwapTarget(String),
    #[error(
        "Multiple matches found for search text: '{0}'. Either provide a more specific search pattern or use replace_all to replace all occurrences."
    )]
    MultipleMatches(String),
    #[error(
        "Match range [{0}..{1}) is out of bounds for content of length {2}. File may have changed externally, consider reading the file again."
    )]
    RangeOutOfBounds(usize, usize, usize),
    #[error("Failed to build fuzzy patch: {message}")]
    PatchBuild { message: String },
}

/// Compute a range from search text, with operation-aware error handling
///
/// Returns Some(range) if a match is found, None if no search or operation
/// doesn't require a match, or an error if a search was provided but no match
/// was found for operations that require it.
fn compute_range(
    source: &str,
    search: Option<&str>,
    operation: &PatchOperation,
) -> Result<Option<Range>, Error> {
    match search {
        Some(s) if !s.is_empty() => {
            let normalized_search = Range::normalize_search_line_endings(source, s);
            let match_result = Range::find_exact(source, &normalized_search)
                .ok_or_else(|| Error::NoMatch(s.to_string()));
            match match_result {
                Ok(r) => Ok(Some(r)),
                Err(e) => {
                    // Handle no match based on operation type
                    match operation {
                        PatchOperation::Replace
                        | PatchOperation::ReplaceAll
                        | PatchOperation::Swap => Err(e),
                        _ => Ok(None),
                    }
                }
            }
        }
        _ => Ok(None),
    }
}

/// A match found in the source text. Represents a range in the source text that
///
/// # Arguments
/// * `haystack` - The original content to patch
/// * `range` - Optional range indicating the location to apply the patch
/// * `operation` - The patch operation to perform
/// * `content` - The content to use for the patch operation
///
/// # Returns
/// The patched content, or an error if the operation fails
fn apply_replacement(
    haystack: String,
    range: Option<Range>,
    operation: &PatchOperation,
    content: &str,
) -> Result<String, Error> {
    let line_ending = Range::detect_line_ending(&haystack);
    let normalized_content = Range::normalize_search_line_endings(&haystack, content);
    // Handle case where range is provided (match found)
    if let Some(patch) = range {
        // Validate the range is within bounds before indexing
        if patch.end() > haystack.len() {
            return Err(Error::RangeOutOfBounds(
                patch.start,
                patch.end(),
                haystack.len(),
            ));
        }

        // Extract the matched text from haystack
        let needle = haystack
            .get(patch.start..patch.end())
            .ok_or_else(|| Error::RangeOutOfBounds(patch.start, patch.end(), haystack.len()))?;

        // Apply the operation based on its type
        match operation {
            // Prepend content before the matched text
            PatchOperation::Prepend => {
                let before = haystack.get(..patch.start).ok_or(Error::RangeOutOfBounds(
                    0,
                    patch.start,
                    haystack.len(),
                ))?;
                let after = haystack.get(patch.start..).ok_or({
                    Error::RangeOutOfBounds(patch.start, haystack.len(), haystack.len())
                })?;
                Ok(format!("{}{}{}", before, normalized_content, after))
            }

            // Replace all occurrences of the matched text with new content
            PatchOperation::ReplaceAll => Ok(haystack.replace(needle, &normalized_content)),

            // Append content after the matched text
            PatchOperation::Append => {
                let before = haystack
                    .get(..patch.end())
                    .ok_or_else(|| Error::RangeOutOfBounds(0, patch.end(), haystack.len()))?;
                let after = haystack.get(patch.end()..).ok_or_else(|| {
                    Error::RangeOutOfBounds(patch.end(), haystack.len(), haystack.len())
                })?;
                Ok(format!(
                    "{}{}{}{}",
                    before, line_ending, normalized_content, after
                ))
            }

            // Replace matched text with new content
            PatchOperation::Replace => {
                // Check if there are multiple matches
                let mut match_count = 0;
                let mut search_start = 0;
                while let Some(pos) = haystack.get(search_start..).and_then(|s| s.find(needle)) {
                    match_count += 1;
                    if match_count > 1 {
                        return Err(Error::MultipleMatches(needle.to_string()));
                    }
                    search_start += pos + needle.len();
                }

                let before = haystack.get(..patch.start).ok_or(Error::RangeOutOfBounds(
                    0,
                    patch.start,
                    haystack.len(),
                ))?;
                let after = haystack.get(patch.end()..).ok_or_else(|| {
                    Error::RangeOutOfBounds(patch.end(), haystack.len(), haystack.len())
                })?;
                Ok(format!("{}{}{}", before, normalized_content, after))
            }

            // Swap with another text in the source
            PatchOperation::Swap => {
                // Find the target text to swap with
                let target_patch = Range::find_exact(&haystack, content)
                    .ok_or_else(|| Error::NoSwapTarget(content.to_string()))?;

                // Handle the case where patches overlap
                if (patch.start <= target_patch.start && patch.end() > target_patch.start)
                    || (target_patch.start <= patch.start && target_patch.end() > patch.start)
                {
                    // For overlapping ranges, we just do an ordinary replacement
                    let before = haystack.get(..patch.start).ok_or(Error::RangeOutOfBounds(
                        0,
                        patch.start,
                        haystack.len(),
                    ))?;
                    let after = haystack.get(patch.end()..).ok_or_else(|| {
                        Error::RangeOutOfBounds(patch.end(), haystack.len(), haystack.len())
                    })?;
                    return Ok(format!("{}{}{}", before, normalized_content, after));
                }

                // We need to handle different ordering of patches
                if patch.start < target_patch.start {
                    // Original text comes first
                    let part1 = haystack.get(..patch.start).ok_or(Error::RangeOutOfBounds(
                        0,
                        patch.start,
                        haystack.len(),
                    ))?;
                    let part2 = haystack
                        .get(patch.end()..target_patch.start)
                        .ok_or_else(|| {
                            Error::RangeOutOfBounds(patch.end(), target_patch.start, haystack.len())
                        })?;
                    let part3 = haystack.get(patch.start..patch.end()).ok_or_else(|| {
                        Error::RangeOutOfBounds(patch.start, patch.end(), haystack.len())
                    })?;
                    let part4 = haystack.get(target_patch.end()..).ok_or_else(|| {
                        Error::RangeOutOfBounds(target_patch.end(), haystack.len(), haystack.len())
                    })?;
                    Ok(format!(
                        "{}{}{}{}{}",
                        part1, normalized_content, part2, part3, part4
                    ))
                } else {
                    // Target text comes first
                    let part1 = haystack.get(..target_patch.start).ok_or({
                        Error::RangeOutOfBounds(0, target_patch.start, haystack.len())
                    })?;
                    let part2 = haystack.get(patch.start..patch.end()).ok_or_else(|| {
                        Error::RangeOutOfBounds(patch.start, patch.end(), haystack.len())
                    })?;
                    let part3 = haystack
                        .get(target_patch.end()..patch.start)
                        .ok_or_else(|| {
                            Error::RangeOutOfBounds(target_patch.end(), patch.start, haystack.len())
                        })?;
                    let part4 = haystack.get(patch.end()..).ok_or_else(|| {
                        Error::RangeOutOfBounds(patch.end(), haystack.len(), haystack.len())
                    })?;
                    Ok(format!(
                        "{}{}{}{}{}",
                        part1, part2, part3, normalized_content, part4
                    ))
                }
            }
        }
    } else {
        // No match (range is None) - treat as empty search (full file operation)
        match operation {
            // Append to the end of the file
            PatchOperation::Append => Ok(format!("{haystack}{line_ending}{normalized_content}")),
            // Prepend to the beginning of the file
            PatchOperation::Prepend => Ok(format!("{normalized_content}{haystack}")),
            // Replace is equivalent to completely replacing the file
            PatchOperation::Replace | PatchOperation::ReplaceAll => Ok(normalized_content),
            // Swap doesn't make sense with empty search - keep source unchanged
            PatchOperation::Swap => Ok(haystack),
        }
    }
}

// Using PatchOperation from forge_domain

// Using FSPatchInput from forge_domain

fn build_fuzzy_patch(
    current_content: &str,
    search_text: &str,
    content: &str,
    patch: TextPatchBlock,
) -> String {
    let _ = (
        Range::normalize_search_line_endings(current_content, search_text),
        Range::normalize_search_line_endings(current_content, content),
        patch.patch,
    );
    patch.patched_text
}

async fn apply_replace_operation<F: TextPatchRepository>(
    infra: &F,
    current_content: String,
    search: &str,
    content: &str,
    operation: &PatchOperation,
) -> Result<String, Error> {
    match compute_range(&current_content, Some(search), operation) {
        Ok(range) => apply_replacement(current_content, range, operation, content),
        Err(Error::NoMatch(search_text))
            if matches!(
                operation,
                PatchOperation::Replace | PatchOperation::ReplaceAll | PatchOperation::Swap
            ) =>
        {
            let normalized_search =
                Range::normalize_search_line_endings(&current_content, &search_text);
            let normalized_content =
                Range::normalize_search_line_endings(&current_content, content);
            let patch = infra
                .build_text_patch(&current_content, &normalized_search, &normalized_content)
                .await
                .map_err(|error| Error::PatchBuild { message: error.to_string() })?;
            Ok(build_fuzzy_patch(
                &current_content,
                &search_text,
                content,
                patch,
            ))
        }
        Err(e) => Err(e),
    }
}

/// Service for patching files with snapshot coordination
///
/// This service coordinates between infrastructure (file I/O) and repository
/// (snapshots) to modify files while preserving the ability to undo changes.
pub struct ForgeFsPatch<F> {
    infra: Arc<F>,
}

impl<F> ForgeFsPatch<F> {
    pub fn new(infra: Arc<F>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<
    F: FileWriterInfra
        + SnapshotRepository
        + ValidationRepository
        + FuzzySearchRepository
        + TextPatchRepository,
> FsPatchService for ForgeFsPatch<F>
{
    async fn patch(
        &self,
        input_path: String,
        search: String,
        content: String,
        replace_all: bool,
    ) -> anyhow::Result<PatchOutput> {
        let path = Path::new(&input_path);
        assert_absolute_path(path)?;

        // Convert replace_all boolean to PatchOperation
        let operation = if replace_all {
            PatchOperation::ReplaceAll
        } else {
            PatchOperation::Replace
        };

        // Read the original content once
        // TODO: use forge_fs
        let mut current_content = fs::read_to_string(path)
            .await
            .map_err(Error::FileOperation)?;

        // Save the old content before modification for diff generation
        let old_content = current_content.clone();

        current_content =
            apply_replace_operation(&*self.infra, current_content, &search, &content, &operation)
                .await?;

        // SNAPSHOT COORDINATION: Always capture snapshot before modifying
        self.infra.insert_snapshot(path).await?;

        // Write final content to file after all patches are applied
        self.infra
            .write(path, Bytes::from(current_content.clone()))
            .await?;

        // Compute hash of the final file content
        let content_hash = compute_hash(&current_content);

        // Validate file syntax using remote validation API (graceful failure)
        let errors = self
            .infra
            .validate_file(path, &current_content)
            .await
            .unwrap_or_default();

        Ok(PatchOutput {
            errors,
            before: old_content,
            after: current_content,
            content_hash,
        })
    }

    async fn multi_patch(
        &self,
        input_path: String,
        edits: Vec<forge_domain::PatchEdit>,
    ) -> anyhow::Result<PatchOutput> {
        let path = Path::new(&input_path);
        assert_absolute_path(path)?;

        // Read the original content once
        let mut current_content = fs::read_to_string(path)
            .await
            .map_err(Error::FileOperation)?;
        // Save the old content before modification for diff generation
        let old_content = current_content.clone();

        // Apply each edit sequentially
        for edit in &edits {
            // Convert replace_all boolean to PatchOperation
            let operation = if edit.replace_all {
                PatchOperation::ReplaceAll
            } else {
                PatchOperation::Replace
            };

            current_content = apply_replace_operation(
                &*self.infra,
                current_content,
                &edit.old_string,
                &edit.new_string,
                &operation,
            )
            .await?;
        }

        // SNAPSHOT COORDINATION: Always capture snapshot before modifying
        self.infra.insert_snapshot(path).await?;

        // Write final content to file after all patches are applied
        self.infra
            .write(path, Bytes::from(current_content.clone()))
            .await?;

        // Compute hash of the final file content
        let content_hash = compute_hash(&current_content);

        // Validate file syntax using remote validation API (graceful failure)
        let errors = self
            .infra
            .validate_file(path, &current_content)
            .await
            .unwrap_or_default();

        Ok(PatchOutput {
            errors,
            before: old_content,
            after: current_content,
            content_hash,
        })
    }
}

#[cfg(test)]
mod tests {
    use forge_app::domain::PatchOperation;
    use forge_domain::SearchMatch;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_range_from_search_match_single_line() {
        let source = "line1\nline2\nline3";
        // 0-based: line 1 (the second line, "line2")
        let search_match = SearchMatch { start_line: 1, end_line: 1 };

        let range = super::Range::from_search_match(source, &search_match);
        let actual = &source[range.start..range.end()];
        let expected = "line2";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_range_from_search_match_multi_line() {
        let source = "line1\nline2\nline3\nline4";
        // 0-based: lines 1-2 (second and third lines, "line2\nline3")
        let search_match = SearchMatch { start_line: 1, end_line: 2 };

        let range = super::Range::from_search_match(source, &search_match);
        let actual = &source[range.start..range.end()];
        let expected = "line2\nline3";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_range_from_search_match_first_line() {
        let source = "line1\nline2\nline3";
        // 0-based: line 0 (first line, "line1")
        let search_match = SearchMatch { start_line: 0, end_line: 0 };

        let range = super::Range::from_search_match(source, &search_match);
        let actual = &source[range.start..range.end()];
        let expected = "line1";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_range_from_search_match_last_line() {
        let source = "line1\nline2\nline3";
        // 0-based: line 2 (third line, "line3")
        let search_match = SearchMatch { start_line: 2, end_line: 2 };

        let range = super::Range::from_search_match(source, &search_match);
        let actual = &source[range.start..range.end()];
        let expected = "line3";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_range_from_search_match_last_line_without_newline() {
        let source = "line1\nline2\nline3"; // No trailing newline
        // 0-based: line 2 (third line, "line3")
        let search_match = SearchMatch { start_line: 2, end_line: 2 };

        let range = super::Range::from_search_match(source, &search_match);
        let actual = &source[range.start..range.end()];
        let expected = "line3";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_range_from_search_match_all_lines() {
        let source = "line1\nline2\nline3";
        // 0-based: lines 0-2 (all three lines)
        let search_match = SearchMatch { start_line: 0, end_line: 2 };

        let range = super::Range::from_search_match(source, &search_match);
        let actual = &source[range.start..range.end()];
        let expected = "line1\nline2\nline3";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_range_from_search_match_empty_source() {
        let source = "";
        // 0-based: line 0 (but source is empty)
        let search_match = SearchMatch { start_line: 0, end_line: 0 };

        let range = super::Range::from_search_match(source, &search_match);
        let actual = &source[range.start..range.end()];
        let expected = "";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_range_from_search_match_single_line_source() {
        let source = "single line";
        // 0-based: line 0 (the only line)
        let search_match = SearchMatch { start_line: 0, end_line: 0 };

        let range = super::Range::from_search_match(source, &search_match);
        let actual = &source[range.start..range.end()];
        let expected = "single line";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_apply_replacement_replace_multiple_matches_error() {
        let source = "test test test";
        let search = Some("test".to_string());
        let operation = PatchOperation::Replace;
        let content = "replaced";

        // Multiple matches error is detected inside apply_replacement, not in
        // compute_range
        let range = super::compute_range(source, search.as_deref(), &operation).unwrap();
        let result = super::apply_replacement(source.to_string(), range, &operation, content);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Multiple matches found for search text: 'test'. Either provide a more specific search pattern or use replace_all to replace all occurrences."));
    }

    #[test]
    fn test_apply_replacement_replace_single_match_success() {
        let source = "hello world test";
        let search = Some("world".to_string());
        let operation = PatchOperation::Replace;
        let content = "universe";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "hello universe test");
    }

    #[test]
    fn test_apply_replacement_prepend() {
        let source = "b\nc\nd";
        let search = Some("b".to_string());
        let operation = PatchOperation::Prepend;
        let content = "a\n".to_string();

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            &content,
        );
        assert_eq!(result.unwrap(), "a\nb\nc\nd");
    }

    #[test]
    fn test_apply_replacement_prepend_empty() {
        let source = "b\nc\nd";
        let search = Some("".to_string());
        let operation = PatchOperation::Prepend;
        let content = "a\n".to_string();

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            &content,
        );
        assert_eq!(result.unwrap(), "a\nb\nc\nd");
    }

    #[test]
    fn test_apply_replacement_prepend_no_search() {
        let source = "hello world";
        let search: Option<String> = None;
        let operation = PatchOperation::Prepend;
        let content = "prefix ";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "prefix hello world");
    }

    #[test]
    fn test_apply_replacement_append() {
        let source = "hello world";
        let search = Some("hello".to_string());
        let operation = PatchOperation::Append;
        let content = " there";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "hello\n there world");
    }

    #[test]
    fn test_apply_replacement_append_no_search() {
        let source = "hello world";
        let search: Option<String> = None;
        let operation = PatchOperation::Append;
        let content = " suffix";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "hello world\n suffix");
    }

    #[test]
    fn test_apply_replacement_replace() {
        let source = "hello world";
        let search = Some("world".to_string());
        let operation = PatchOperation::Replace;
        let content = "universe";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "hello universe");
    }

    #[test]
    fn test_apply_replacement_replace_no_search() {
        let source = "hello world";
        let search: Option<String> = None;
        let operation = PatchOperation::Replace;
        let content = "new content";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "new content");
    }

    #[test]
    fn test_apply_replacement_swap() {
        let source = "apple banana cherry";
        let search = Some("apple".to_string());
        let operation = PatchOperation::Swap;
        let content = "banana";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "banana apple cherry");
    }

    #[test]
    fn test_apply_replacement_swap_reverse_order() {
        let source = "apple banana cherry";
        let search = Some("banana".to_string());
        let operation = PatchOperation::Swap;
        let content = "apple";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "banana apple cherry");
    }

    #[test]
    fn test_apply_replacement_swap_overlapping() {
        let source = "abcdef";
        let search = Some("abc".to_string());
        let operation = PatchOperation::Swap;
        let content = "cde";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "cdedef");
    }

    #[test]
    fn test_apply_replacement_swap_no_search() {
        let source = "hello world";
        let search: Option<String> = None;
        let operation = PatchOperation::Swap;
        let content = "anything";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "hello world");
    }

    #[test]
    fn test_apply_replacement_multiline() {
        let source = "line1\nline2\nline3";
        let search = Some("line2".to_string());
        let operation = PatchOperation::Replace;
        let content = "replaced_line";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "line1\nreplaced_line\nline3");
    }

    #[test]
    fn test_apply_replacement_with_special_chars() {
        let source = "hello $world @test";
        let search = Some("$world".to_string());
        let operation = PatchOperation::Replace;
        let content = "$universe";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "hello $universe @test");
    }

    #[test]
    fn test_apply_replacement_empty_content() {
        let source = "hello world test";
        let search = Some("world ".to_string());
        let operation = PatchOperation::Replace;
        let content = "";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "hello test");
    }

    #[test]
    fn test_apply_replacement_first_occurrence_only() {
        let source = "test test test";
        let search = Some("test".to_string());
        let operation = PatchOperation::Replace;
        let content = "replaced";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Multiple matches found for search text: 'test'")
        );
    }

    // Error cases
    #[test]
    fn test_apply_replacement_no_match() {
        let source = "hello world";
        let search = Some("missing".to_string());
        let operation = PatchOperation::Replace;
        let _content = "replacement";

        let range = super::compute_range(source, search.as_deref(), &operation);
        assert!(range.is_err());
        assert!(
            range
                .unwrap_err()
                .to_string()
                .contains("Could not find match for search text: 'missing'")
        );
    }

    #[test]
    fn test_apply_replacement_swap_no_target() {
        let source = "hello world";
        let search = Some("hello".to_string());
        let operation = PatchOperation::Swap;
        let content = "missing";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Could not find swap target text: missing")
        );
    }

    #[test]
    fn test_apply_replacement_edge_case_same_text() {
        let source = "hello hello";
        let search = Some("hello".to_string());
        let operation = PatchOperation::Swap;
        let content = "hello";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "hello hello");
    }

    #[test]
    fn test_apply_replacement_whitespace_handling() {
        let source = "  hello   world  ";
        let search = Some("hello   world".to_string());
        let operation = PatchOperation::Replace;
        let content = "test";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "  test  ");
    }

    #[test]
    fn test_apply_replacement_unicode() {
        let source = "héllo wørld 🌍";
        let search = Some("wørld".to_string());
        let operation = PatchOperation::Replace;
        let content = "univérse";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "héllo univérse 🌍");
    }

    #[test]
    fn test_apply_replacement_replace_all_multiple_occurrences() {
        let source = "test test test";
        let search = Some("test".to_string());
        let operation = PatchOperation::ReplaceAll;
        let content = "replaced";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "replaced replaced replaced");
    }

    #[test]
    fn test_apply_replacement_replace_all_no_search() {
        let source = "hello world";
        let search: Option<String> = None;
        let operation = PatchOperation::ReplaceAll;
        let content = "new content";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "new content");
    }

    #[test]
    fn test_apply_replacement_replace_all_empty_search() {
        let source = "hello world";
        let search = Some("".to_string());
        let operation = PatchOperation::ReplaceAll;
        let content = "new content";

        let result = super::apply_replacement(
            source.to_string(),
            super::compute_range(source, search.as_deref(), &operation).unwrap(),
            &operation,
            content,
        );
        assert_eq!(result.unwrap(), "new content");
    }

    #[test]
    fn test_apply_replacement_replace_all_no_match() {
        let source = "hello world";
        let search = Some("missing".to_string());
        let operation = PatchOperation::ReplaceAll;
        let _content = "replacement";

        let range = super::compute_range(source, search.as_deref(), &operation);
        assert!(range.is_err());
        assert!(
            range
                .unwrap_err()
                .to_string()
                .contains("Could not find match for search text: 'missing'")
        );
    }

    #[test]
    fn test_range_from_search_match_crlf_single_line() {
        let source = "line1\r\nline2\r\nline3";
        // 0-based: line 1 (the second line, "line2")
        let search_match = SearchMatch { start_line: 1, end_line: 1 };

        let range = super::Range::from_search_match(source, &search_match);
        let actual = &source[range.start..range.end()];
        let expected = "line2";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_range_from_search_match_crlf_multi_line() {
        let source = "line1\r\nline2\r\nline3\r\nline4";
        // 0-based: lines 1-2 (second and third lines, "line2\r\nline3")
        let search_match = SearchMatch { start_line: 1, end_line: 2 };

        let range = super::Range::from_search_match(source, &search_match);
        let actual = &source[range.start..range.end()];
        let expected = "line2\r\nline3";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_range_from_search_match_crlf_first_line() {
        let source = "line1\r\nline2\r\nline3";
        // 0-based: line 0 (first line, "line1")
        let search_match = SearchMatch { start_line: 0, end_line: 0 };

        let range = super::Range::from_search_match(source, &search_match);
        let actual = &source[range.start..range.end()];
        let expected = "line1";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_range_from_search_match_crlf_all_lines() {
        let source = "line1\r\nline2\r\nline3";
        // 0-based: lines 0-2 (all three lines)
        let search_match = SearchMatch { start_line: 0, end_line: 2 };

        let range = super::Range::from_search_match(source, &search_match);
        let actual = &source[range.start..range.end()];
        let expected = "line1\r\nline2\r\nline3";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_detect_line_ending_crlf() {
        let source = "line1\r\nline2\r\nline3";
        let line_ending = super::Range::detect_line_ending(source);
        assert_eq!(line_ending, "\r\n");
    }

    #[test]
    fn test_detect_line_ending_lf() {
        let source = "line1\nline2\nline3";
        let line_ending = super::Range::detect_line_ending(source);
        assert_eq!(line_ending, "\n");
    }

    #[test]
    fn test_compute_range_normalizes_search_crlf() {
        let source = "line1\r\nline2\r\nline3";
        let search = Some("line2\nline3".to_string());
        let operation = PatchOperation::Replace;

        let range = super::compute_range(source, search.as_deref(), &operation).unwrap();
        let actual = &source[range.unwrap().start..range.unwrap().end()];
        let expected = "line2\r\nline3";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_compute_range_normalizes_search_lf() {
        let source = "line1\nline2\nline3";
        let search = Some("line2\r\nline3".to_string());
        let operation = PatchOperation::Replace;

        let range = super::compute_range(source, search.as_deref(), &operation).unwrap();
        let actual = &source[range.unwrap().start..range.unwrap().end()];
        let expected = "line2\nline3";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_compute_range_normalizes_search_crlf_input() {
        let source = "line1\r\nline2\r\nline3";
        let search = Some("line2\r\nline3".to_string());
        let operation = PatchOperation::Replace;

        let range = super::compute_range(source, search.as_deref(), &operation).unwrap();
        let actual = &source[range.unwrap().start..range.unwrap().end()];
        let expected = "line2\r\nline3";

        assert_eq!(actual, expected);
    }

    // --- Out-of-bounds safety tests ---

    #[test]
    fn test_range_from_search_match_out_of_bounds_start_line() {
        let source = "line1\nline2\nline3";
        // start_line way past end of file
        let search_match = SearchMatch { start_line: 100, end_line: 200 };

        let range = super::Range::from_search_match(source, &search_match);
        // Should not panic; range should be clamped so it doesn't exceed source
        assert!(range.end() <= source.len());
    }

    #[test]
    fn test_range_from_search_match_end_line_past_eof() {
        let source = "line1\nline2\nline3";
        // start_line valid, end_line past end
        let search_match = SearchMatch { start_line: 1, end_line: 100 };

        let range = super::Range::from_search_match(source, &search_match);
        assert!(range.end() <= source.len());
        // Should include from line2 to end of source
        let actual = &source[range.start..range.end()];
        assert!(actual.contains("line2"));
        assert!(actual.contains("line3"));
    }

    #[test]
    fn test_range_from_search_match_trailing_newline() {
        let source = "line1\nline2\nline3\n"; // trailing newline
        let search_match = SearchMatch { start_line: 2, end_line: 2 };

        let range = super::Range::from_search_match(source, &search_match);
        assert!(range.end() <= source.len());
        let actual = &source[range.start..range.end()];
        assert_eq!(actual, "line3");
    }

    #[test]
    fn test_range_from_search_match_unicode_content() {
        let source = "héllo\nwørld\n🌍";
        let search_match = SearchMatch { start_line: 1, end_line: 1 };

        let range = super::Range::from_search_match(source, &search_match);
        assert!(range.end() <= source.len());
        let actual = &source[range.start..range.end()];
        assert_eq!(actual, "wørld");
    }

    #[test]
    fn test_range_from_search_match_unicode_multiline() {
        let source = "héllo\nwørld\n🌍";
        let search_match = SearchMatch { start_line: 0, end_line: 2 };

        let range = super::Range::from_search_match(source, &search_match);
        assert!(range.end() <= source.len());
        let actual = &source[range.start..range.end()];
        assert_eq!(actual, source);
    }

    #[test]
    fn test_range_from_search_match_mixed_line_endings() {
        let source = "line1\r\nline2\nline3";
        let search_match = SearchMatch { start_line: 1, end_line: 1 };

        let range = super::Range::from_search_match(source, &search_match);
        assert!(range.end() <= source.len());
        let actual = &source[range.start..range.end()];
        assert_eq!(actual, "line2");
    }

    #[test]
    fn test_apply_replacement_with_out_of_bounds_range_returns_error() {
        let source = "short";
        // Simulate a bad range that exceeds source length
        let bad_range = Some(super::Range::new(0, 1000));
        let operation = PatchOperation::Replace;
        let content = "replacement";

        let result = super::apply_replacement(source.to_string(), bad_range, &operation, content);
        assert!(result.is_err());
    }
}
