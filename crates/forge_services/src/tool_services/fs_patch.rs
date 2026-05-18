use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;
use forge_app::domain::PatchOperation;
use forge_app::{EnvironmentInfra, FileWriterInfra, FsPatchService, PatchOutput, compute_hash};
use forge_config::ForgeConfig;
use forge_domain::{
    FuzzySearchRepository, SearchMatch, SnapshotRepository, TextPatchBlock, TextPatchRepository,
    ValidationRepository,
};
use thiserror::Error;
use tokio::fs;
use similar::{ChangeTag, TextDiff};

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
    #[error(
        "Overlapping edits: edit #{0} (bytes {1} to {2}) overlaps with edit #{3} (bytes {4} to {5})"
    )]
    OverlappingEdits(usize, usize, usize, usize, usize, usize),
    #[error(
        "Edit #{0} failed: old_string not found in file '{1}'\n\nSearched for:\n```\n{2}\n```\n\nTip: The file may have changed since you started. Try reading the file again."
    )]
    EditNotFound(usize, String, String),
    #[error(
        "Edit #{0} failed: Multiple matches for '{1}' (found {2}).\nEither add more context to make the match unique, or set replace_all: true."
    )]
    MultipleMatchesInEdit(usize, String, usize),
}

const FUZZY_THRESHOLD: f64 = 0.90;

fn truncate_for_error(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

fn byte_to_line_column(content: &str, byte_pos: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (i, c) in content.char_indices() {
        if i >= byte_pos {
            return (line, col);
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn line_range_to_byte_range(
    content: &str,
    start_line: usize,
    end_line: usize,
) -> (usize, usize) {
    let lines: Vec<&str> = content.lines().collect();

    let start_byte: usize = lines[..start_line]
        .iter()
        .map(|l| l.len() + 1)
        .sum();

    let end_byte: usize = lines[..=end_line]
        .iter()
        .map(|l| l.len() + 1)
        .sum();

    (start_byte, end_byte - start_byte)
}

fn find_all_matches(content: &str, search: &str) -> Vec<(usize, usize)> {
    let mut matches = Vec::new();
    let mut search_start = 0;

    while let Some(pos) = content[search_start..].find(search) {
        let actual_pos = search_start + pos;
        matches.push((actual_pos, search.len()));
        search_start = actual_pos + search.len();
    }

    matches
}

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn find_whitespace_normalized_matches(
    content: &str,
    old_string: &str,
) -> Vec<(usize, usize)> {
    let content_lines: Vec<&str> = content.lines().collect();
    let old_lines: Vec<&str> = old_string.lines().collect();

    if old_lines.is_empty() || content_lines.len() < old_lines.len() {
        return Vec::new();
    }

    let norm_content: Vec<String> = content_lines
        .iter()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    let norm_old: Vec<String> = old_lines
        .iter()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();

    let mut matches = Vec::new();
    for i in 0..=content_lines.len().saturating_sub(old_lines.len()) {
        let window = &norm_content[i..i + old_lines.len()];
        let all_match = window
            .iter()
            .zip(norm_old.iter())
            .all(|(c, o)| c == o);

        if all_match {
            let start_line = i;
            let end_line = i + old_lines.len() - 1;
            let (position, length) = line_range_to_byte_range(content, start_line, end_line);
            matches.push((position, length));
        }
    }
    matches
}

fn fuzzy_find(content: &str, old_string: &str) -> Option<(usize, usize)> {
    let old_lines: Vec<&str> = old_string.lines().collect();
    let content_lines: Vec<&str> = content.lines().collect();

    if old_lines.is_empty() || content_lines.len() < old_lines.len() {
        return None;
    }

    let mut best_ratio = 0.0f64;
    let mut best_start_line = 0;

    for i in 0..=content_lines.len().saturating_sub(old_lines.len()) {
        let candidate_lines = &content_lines[i..i + old_lines.len()];
        let candidate = candidate_lines.join("\n");

        let diff = TextDiff::from_lines(old_string, &candidate);
        let mut equal = 0;
        let mut total = 0;

        for change in diff.iter_all_changes() {
            if matches!(change.tag(), ChangeTag::Equal) {
                equal += 1;
            }
            total += 1;
        }

        let ratio = if total > 0 {
            equal as f64 / total as f64
        } else {
            0.0
        };
        if ratio > best_ratio {
            best_ratio = ratio;
            best_start_line = i;
        }
    }

    if best_ratio >= FUZZY_THRESHOLD {
        let byte_pos: usize = content_lines[..best_start_line]
            .iter()
            .map(|l| l.len() + 1)
            .sum();

        return Some((byte_pos, old_string.len()));
    }

    None
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

async fn apply_fuzzy_search_fallback<F: FuzzySearchRepository>(
    infra: &F,
    current_content: String,
    search_text: String,
    content: &str,
    operation: &PatchOperation,
) -> Result<String, Error> {
    let range = match infra
        .fuzzy_search(&search_text, &current_content, false)
        .await
    {
        Ok(matches) if !matches.is_empty() => matches
            .first()
            .map(|m| Range::from_search_match(&current_content, m)),
        _ => return Err(Error::NoMatch(search_text)),
    };

    apply_replacement(current_content, range, operation, content)
}

async fn apply_text_patch_fallback<F: TextPatchRepository>(
    infra: &F,
    current_content: String,
    search_text: String,
    content: &str,
) -> Result<String, Error> {
    let normalized_search = Range::normalize_search_line_endings(&current_content, &search_text);
    let normalized_content = Range::normalize_search_line_endings(&current_content, content);
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

async fn apply_replace_operation<F: FuzzySearchRepository + TextPatchRepository>(
    infra: &F,
    current_content: String,
    search: &str,
    content: &str,
    operation: &PatchOperation,
    use_text_patch_fallback: bool,
) -> Result<String, Error> {
    match compute_range(&current_content, Some(search), operation) {
        Ok(range) => apply_replacement(current_content, range, operation, content),
        Err(Error::NoMatch(search_text))
            if matches!(
                operation,
                PatchOperation::Replace | PatchOperation::ReplaceAll | PatchOperation::Swap
            ) =>
        {
            if use_text_patch_fallback {
                apply_text_patch_fallback(infra, current_content, search_text, content).await
            } else {
                apply_fuzzy_search_fallback(infra, current_content, search_text, content, operation)
                    .await
            }
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
    F: EnvironmentInfra<Config = ForgeConfig>
        + FileWriterInfra
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
        let use_text_patch_fallback = self.infra.get_config()?.use_text_patch_fallback;

        current_content = apply_replace_operation(
            &*self.infra,
            current_content,
            &search,
            &content,
            &operation,
            use_text_patch_fallback,
        )
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

        let original_content = fs::read_to_string(path)
            .await
            .map_err(Error::FileOperation)?;
        let old_content = original_content.clone();
        let use_text_patch_fallback = self.infra.get_config()?.use_text_patch_fallback;

        #[derive(Clone)]
        struct PositionedEdit {
            index: usize,
            position: usize,
            old_len: usize,
            edit: forge_domain::PatchEdit,
        }

        let mut positioned_edits: Vec<PositionedEdit> = Vec::new();

        for (index, edit) in edits.iter().enumerate() {
            if edit.old_string.is_empty() {
                return Err(anyhow::anyhow!(
                    "Edit #{} failed: old_string cannot be empty",
                    index + 1
                ));
            }

            let exact_matches = find_all_matches(&original_content, &edit.old_string);

            match exact_matches.as_slice() {
                [] => {
                    let ws_matches = find_whitespace_normalized_matches(&original_content, &edit.old_string);
                    match ws_matches.as_slice() {
                        [(pos, len)] => {
                            positioned_edits.push(PositionedEdit {
                                index,
                                position: *pos,
                                old_len: *len,
                                edit: edit.clone(),
                            });
                        }
                        [] => {
                            if let Some((pos, len)) = fuzzy_find(&original_content, &edit.old_string) {
                                positioned_edits.push(PositionedEdit {
                                    index,
                                    position: pos,
                                    old_len: len,
                                    edit: edit.clone(),
                                });
                            } else {
                                return Err(anyhow::anyhow!(
                                    "Edit #{} failed: old_string not found in file '{}'\n\nSearched for:\n```\n{}\n```\n\nTip: The file may have changed since you started. Try reading the file again.",
                                    index + 1,
                                    path.display(),
                                    truncate_for_error(&edit.old_string, 200)
                                ));
                            }
                        }
                        multiple => {
                            if edit.replace_all {
                                for (pos, len) in multiple {
                                    positioned_edits.push(PositionedEdit {
                                        index,
                                        position: *pos,
                                        old_len: *len,
                                        edit: edit.clone(),
                                    });
                                }
                            } else {
                                return Err(anyhow::anyhow!(
                                    "Edit #{} failed: Multiple matches for '{}' (found {}).\nEither add more context to make the match unique, or set replace_all: true.",
                                    index + 1,
                                    truncate_for_error(&edit.old_string, 100),
                                    multiple.len()
                                ));
                            }
                        }
                    }
                }
                [(pos, len)] => {
                    positioned_edits.push(PositionedEdit {
                        index,
                        position: *pos,
                        old_len: *len,
                        edit: edit.clone(),
                    });
                }
                multiple => {
                    if edit.replace_all {
                        for (pos, len) in multiple {
                            positioned_edits.push(PositionedEdit {
                                index,
                                position: *pos,
                                old_len: *len,
                                edit: edit.clone(),
                            });
                        }
                    } else {
                        return Err(anyhow::anyhow!(
                            "Edit #{} failed: Multiple matches for '{}' (found {}).\nEither add more context to make the match unique, or set replace_all: true.",
                            index + 1,
                            truncate_for_error(&edit.old_string, 100),
                            multiple.len()
                        ));
                    }
                }
            }
        }

        positioned_edits.sort_by(|a, b| b.position.cmp(&a.position));

        let mut sorted_for_overlap = positioned_edits.clone();
        sorted_for_overlap.sort_by_key(|e| e.position);
        for window in sorted_for_overlap.windows(2) {
            let (a, b) = (&window[0], &window[1]);
            let a_end = a.position + a.old_len;
            if a_end > b.position {
                return Err(anyhow::anyhow!(
                    "Overlapping edits: edit #{} (bytes {} to {}) overlaps with edit #{} (bytes {} to {})",
                    a.index + 1, a.position, a_end,
                    b.index + 1, b.position, b.position + b.old_len
                ));
            }
        }

        let mut current_content = original_content.clone();
        for plan in &positioned_edits {
            let operation = if plan.edit.replace_all {
                PatchOperation::ReplaceAll
            } else {
                PatchOperation::Replace
            };

            current_content = apply_replace_operation(
                &*self.infra,
                current_content,
                &plan.edit.old_string,
                &plan.edit.new_string,
                &operation,
                use_text_patch_fallback,
            )
            .await?;
        }

        self.infra.insert_snapshot(path).await?;

        let temp_path = path.with_extension("tmp");
        fs::write(&temp_path, &current_content).await?;
        fs::rename(&temp_path, path).await?;

        let verification = fs::read_to_string(path).await?;
        if verification != current_content {
            fs::write(path, &original_content).await?;
            return Err(anyhow::anyhow!(
                "Write verification failed after atomic write, original restored"
            ));
        }

        let content_hash = compute_hash(&current_content);

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
    fn test_apply_replace_operation_uses_fuzzy_search_when_text_patch_fallback_disabled() {
        let fixture = tokio::runtime::Runtime::new().unwrap();

        let actual = fixture.block_on(super::apply_replace_operation(
            &FallbackRepository,
            "alpha\nbeta\ngamma".to_string(),
            "betaa",
            "delta",
            &PatchOperation::Replace,
            false,
        ));

        let expected = "alpha\ndelta\ngamma";
        assert_eq!(actual.unwrap(), expected);
    }

    #[test]
    fn test_apply_replace_operation_uses_text_patch_when_enabled() {
        let fixture = tokio::runtime::Runtime::new().unwrap();

        let actual = fixture.block_on(super::apply_replace_operation(
            &FallbackRepository,
            "alpha\nbeta\ngamma".to_string(),
            "betaa",
            "delta",
            &PatchOperation::Replace,
            true,
        ));

        let expected = "patched via text patch";
        assert_eq!(actual.unwrap(), expected);
    }

    #[derive(Default)]
    struct FallbackRepository;

    #[async_trait::async_trait]
    impl forge_domain::FuzzySearchRepository for FallbackRepository {
        async fn fuzzy_search(
            &self,
            _needle: &str,
            _haystack: &str,
            _search_all: bool,
        ) -> anyhow::Result<Vec<forge_domain::SearchMatch>> {
            let actual = vec![forge_domain::SearchMatch { start_line: 1, end_line: 1 }];
            Ok(actual)
        }
    }

    #[async_trait::async_trait]
    impl forge_domain::TextPatchRepository for FallbackRepository {
        async fn build_text_patch(
            &self,
            _haystack: &str,
            _old_string: &str,
            _new_string: &str,
        ) -> anyhow::Result<forge_domain::TextPatchBlock> {
            let actual = forge_domain::TextPatchBlock {
                patch: "@@ -1 +1 @@".to_string(),
                patched_text: "patched via text patch".to_string(),
            };
            Ok(actual)
        }
    }

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

    // ================================================
    // FIX: multi_patch byte offset corruption tests
    // Tests for the fix that sorts edits by position
    // ================================================

    #[test]
    fn test_multi_patch_sequential_edits_no_offset_corruption() {
        // This test verifies that sequential edits don't corrupt byte offsets
        // when edits are applied from bottom-to-top (sorted by position descending)

        let source = "let start = this.started_at;\nlet end = this.ended_at;\nlet result = self.calculate();";
        let original = source.to_string();

        // Test Case 1: Two sequential edits
        // Edit 1: Replace "this.started_at" → "Instant::now()" at position ~13
        // Edit 2: Replace "this.ended_at" → "Instant::now()" at position ~41

        // Verify that we can find both strings in the original
        let range1 = super::Range::find_exact(source, "this.started_at");
        let range2 = super::Range::find_exact(source, "this.ended_at");

        assert!(range1.is_some(), "Should find first match");
        assert!(range2.is_some(), "Should find second match");

        // range2 should come after range1
        assert!(range2.unwrap().start > range1.unwrap().start,
            "Second match should come after first");

        // When sorted descending (bottom-to-top), range2 is applied first
        // This doesn't affect range1's position since range2 is after it
        let mut edits: Vec<(usize, &str)> = vec![
            (range1.unwrap().start, "this.started_at"),
            (range2.unwrap().start, "this.ended_at"),
        ];
        edits.sort_by(|a, b| b.0.cmp(&a.0)); // Descending

        // Apply edits in sorted order
        let mut result = original.clone();
        for (_, search) in edits {
            let replacement = if search == "this.started_at" {
                "Instant::now()"
            } else {
                "Instant::now()"
            };
            result = result.replace(search, replacement);
        }

        // Verify both replacements happened correctly
        assert!(result.contains("let start = Instant::now();"),
            "First replacement should succeed");
        assert!(result.contains("let end = Instant::now();"),
            "Second replacement should succeed");
    }

    #[test]
    fn test_multi_patch_different_length_replacements() {
        // Test that replacing text of different lengths doesn't corrupt offsets

        let source = "alpha\nbeta\ngamma\ndelta";
        let original = source.to_string();

        // Find positions
        let range_alpha = super::Range::find_exact(source, "alpha").unwrap();
        let range_delta = super::Range::find_exact(source, "delta").unwrap();

        // Replace "alpha" (5 chars) with "ALPHA_REPLACED_WITH_LONG_TEXT" (28 chars)
        // Replace "delta" (5 chars) with "DELTA" (5 chars)

        let mut edits: Vec<(usize, &str, &str)> = vec![
            (range_alpha.start, "alpha", "ALPHA_REPLACED_WITH_LONG_TEXT"),
            (range_delta.start, "delta", "DELTA"),
        ];
        edits.sort_by(|a, b| b.0.cmp(&a.0)); // Descending

        let mut result = original.clone();
        for (_, search, replacement) in edits {
            result = result.replacen(search, replacement, 1);
        }

        // Verify delta wasn't affected by alpha's expansion
        assert!(result.contains("ALPHA_REPLACED_WITH_LONG_TEXT"),
            "Alpha should be replaced with longer text");
        assert!(result.contains("DELTA"),
            "Delta should be replaced correctly");
        assert!(result.contains("gamma"),
            "Gamma should remain unchanged");
        assert!(result.contains("beta"),
            "Beta should remain unchanged");
    }

    #[test]
    fn test_multi_patch_overlapping_ranges_handled() {
        // Test that overlapping search patterns are handled correctly

        let source = "aaa bbb aaa";
        let original = source.to_string();

        // Find positions
        let range1 = super::Range::find_exact(source, "aaa").unwrap();
        let range2 = super::Range::find_exact(source, "bbb").unwrap();

        let mut edits: Vec<(usize, &str, &str)> = vec![
            (range1.start, "aaa", "XXX"),
            (range2.start, "bbb", "YYY"),
        ];
        edits.sort_by(|a, b| b.0.cmp(&a.0));

        let mut result = original.clone();
        for (_, search, replacement) in edits {
            result = result.replacen(search, replacement, 1);
        }

        // Both should be replaced
        assert_eq!(result, "XXX bbb XXX".replace("bbb", "YYY"));
    }

    #[test]
    fn test_multi_patch_order_preservation() {
        // Verify that the sorting preserves correct replacement order

        let source = "AAAA\nBBBB\nCCCC";
        let original = source.to_string();

        let ranges: Vec<(usize, &str)> = vec![
            (super::Range::find_exact(source, "AAAA").unwrap().start, "AAAA"),
            (super::Range::find_exact(source, "BBBB").unwrap().start, "BBBB"),
            (super::Range::find_exact(source, "CCCC").unwrap().start, "CCCC"),
        ];

        // Sort descending
        let mut sorted = ranges.clone();
        sorted.sort_by(|a, b| b.0.cmp(&a.0));

        // After sorting descending, CCCC should be first, then BBBB, then AAAA
        assert_eq!(sorted[0].1, "CCCC");
        assert_eq!(sorted[1].1, "BBBB");
        assert_eq!(sorted[2].1, "AAAA");
    }

    #[test]
    fn test_multi_patch_multiple_same_line_edits() {
        // Test multiple edits on the same line - most dangerous case

        let source = "let x = a; let y = b; let z = c;";
        let original = source.to_string();

        // Find all three positions
        let range_a = super::Range::find_exact(source, "a").unwrap();
        let range_b = super::Range::find_exact(source, "b").unwrap();
        let range_c = super::Range::find_exact(source, "c").unwrap();

        let mut edits: Vec<(usize, &str, &str)> = vec![
            (range_a.start, "a", "1"),
            (range_b.start, "b", "2"),
            (range_c.start, "c", "3"),
        ];
        edits.sort_by(|a, b| b.0.cmp(&a.0));

        let mut result = original.clone();
        for (_, search, replacement) in edits {
            result = result.replacen(search, replacement, 1);
        }

        // All replacements should succeed
        assert_eq!(result, "let x = 1; let y = 2; let z = 3;");
    }

    #[test]
    fn test_multi_patch_real_world_async_spawn_scenario() {
        // Reproduce the real-world bug from GitHub issue #3249
        // The issue was: async move { let start = this.started_at; }
        // After patch: async mov let start = Instant::now();

        let source = "let handle = tokio::spawn(async move {\n let start = this.started_at;\n});";

        // Simulate two edits that would cause the bug:
        // Edit 1: The "async move {" search pattern
        // Edit 2: "this.started_at" replacement

        let range1 = super::Range::find_exact(source, "async move {").unwrap();
        let range2 = super::Range::find_exact(source, "let start = this.started_at;").unwrap();

        // Verify ranges don't overlap
        assert!(range1.end() < range2.start,
            "First edit should come before second edit");

        // Sort descending (bottom-to-top)
        let mut edits: Vec<(usize, &str, &str)> = vec![
            (range1.start, "async move {", "async move {"),
            (range2.start, "this.started_at", "Instant::now()"),
        ];
        edits.sort_by(|a, b| b.0.cmp(&a.0));

        // Apply
        let mut result = source.to_string();
        for (_, search, replacement) in edits {
            result = result.replace(search, replacement);
        }

        // Verify the async block is intact
        assert!(result.contains("async move {"),
            "async move block should be intact");
        assert!(result.contains("let start = Instant::now();"),
            "start replacement should work");
        assert!(result.contains("});"),
            "closing brace should be intact");
    }

    #[test]
    fn test_multi_patch_zero_length_edit_at_end() {
        // Test editing at the very end of the file

        let source = "line1\nline2\nline3";
        let original = source.to_string();

        let range = super::Range::find_exact(source, "line3").unwrap();

        // Edit at the end should work regardless of sorting
        let mut edits: Vec<(usize, &str, &str)> = vec![
            (range.start, "line3", "LINE3"),
        ];
        edits.sort_by(|a, b| b.0.cmp(&a.0));

        let mut result = original.clone();
        for (_, search, replacement) in edits {
            result = result.replace(search, replacement);
        }

        assert_eq!(result, "line1\nline2\nLINE3");
    }

    #[test]
    fn test_multi_patch_consecutive_edits() {
        // Test edits that are directly next to each other

        let source = "abcdef";
        let original = source.to_string();

        // abc and def are adjacent
        let range_abc = super::Range::find_exact(source, "abc").unwrap();
        let range_def = super::Range::find_exact(source, "def").unwrap();

        // After sorting descending: def first, then abc
        let mut edits: Vec<(usize, &str, &str)> = vec![
            (range_abc.start, "abc", "ABC"),
            (range_def.start, "def", "DEF"),
        ];
        edits.sort_by(|a, b| b.0.cmp(&a.0));

        let mut result = original.clone();
        for (_, search, replacement) in edits {
            result = result.replace(search, replacement);
        }

        assert_eq!(result, "ABCDEF");
    }

    // ================================================
    // PHASE 1: Safety Critical Tests
    // ================================================

    #[test]
    fn test_overlap_detection_rejects_overlapping() {
        let edits = vec![
            (0, 10, "0123456789"),   // edit 1: bytes 0-10
            (5, 15, "5678901234567890"), // edit 2: bytes 5-20 - OVERLAPS!
        ];

        let mut sorted = edits.clone();
        sorted.sort_by_key(|e| e.0);

        let mut has_overlap = false;
        for window in sorted.windows(2) {
            let (a_start, a_len, _) = window[0];
            let (b_start, _, _) = window[1];
            let a_end = a_start + a_len;
            if a_end > b_start {
                has_overlap = true;
                break;
            }
        }

        assert!(has_overlap, "Should detect overlapping edits");
    }

    #[test]
    fn test_overlap_detection_accepts_non_overlapping() {
        let edits = vec![
            (0, 5, "01234"),
            (10, 5, "ABCDE"), // Doesn't overlap
        ];

        let mut sorted = edits.clone();
        sorted.sort_by_key(|e| e.0);

        let mut has_overlap = false;
        for window in sorted.windows(2) {
            let (a_start, a_len, _) = window[0];
            let (b_start, _, _) = window[1];
            let a_end = a_start + a_len;
            if a_end > b_start {
                has_overlap = true;
                break;
            }
        }

        assert!(!has_overlap, "Should not detect overlap for non-overlapping edits");
    }

    #[test]
    fn test_adjacent_edits_accepted() {
        let edits = vec![
            (0, 5, "01234"),
            (5, 5, "ABCDE"), // Touches but doesn't overlap
        ];

        let mut sorted = edits.clone();
        sorted.sort_by_key(|e| e.0);

        let mut has_overlap = false;
        for window in sorted.windows(2) {
            let (a_start, a_len, _) = window[0];
            let (b_start, _, _) = window[1];
            let a_end = a_start + a_len;
            if a_end > b_start {
                has_overlap = true;
                break;
            }
        }

        assert!(!has_overlap, "Adjacent edits should be accepted");
    }

    #[test]
    fn test_unique_match_accepted() {
        let content = "hello world";
        let matches = super::find_all_matches(content, "world");
        assert_eq!(matches.len(), 1, "Should find exactly one match");
    }

    #[test]
    fn test_duplicate_match_rejected() {
        let content = "let x = 1; let x = 1;";
        let matches = super::find_all_matches(content, "let x = 1;");
        assert_eq!(matches.len(), 2, "Should find two matches");
    }

    #[test]
    fn test_replace_all_with_multiple_matches() {
        let content = "test test test";
        let result = content.replace("test", "REPLACED");
        assert_eq!(result, "REPLACED REPLACED REPLACED");
    }

    #[test]
    fn test_truncate_for_error_short() {
        let s = "hello";
        let result = super::truncate_for_error(s, 10);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_for_error_long() {
        let s = "this is a very long string";
        let result = super::truncate_for_error(s, 10);
        assert_eq!(result.len(), 10);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_byte_to_line_column_simple() {
        let content = "line1\nline2\nline3";
        let (line, col) = super::byte_to_line_column(content, 0);
        assert_eq!((line, col), (1, 1));

        let (line, col) = super::byte_to_line_column(content, 6);
        assert_eq!((line, col), (2, 1));
    }

    #[test]
    fn test_line_range_to_byte_range() {
        let content = "line1\nline2\nline3";
        let (start, len) = super::line_range_to_byte_range(content, 0, 0);
        assert_eq!(start, 0);
        assert_eq!(len, 5);

        let (start, len) = super::line_range_to_byte_range(content, 1, 1);
        assert_eq!(start, 6);
        assert_eq!(len, 5);
    }

    // ================================================
    // PHASE 2: Whitespace Normalization Tests
    // ================================================

    #[test]
    fn test_whitespace_normalized_match_succeeds() {
        let content = "fn  main()  {";
        let old_string = "fn main() {";

        let matches = super::find_whitespace_normalized_matches(content, old_string);
        assert_eq!(matches.len(), 1, "Should find whitespace-normalized match");
    }

    #[test]
    fn test_whitespace_preserves_original() {
        let content = "fn  main()  {";
        let old_string = "fn main() {";

        let matches = super::find_whitespace_normalized_matches(content, old_string);
        if let Some((pos, len)) = matches.first() {
            let matched = &content[*pos..*pos + *len];
            assert_eq!(matched, "fn  main()  {", "Should preserve original whitespace");
        }
    }

    #[test]
    fn test_whitespace_multi_line() {
        let content = "fn main() {\n    let x = 1;\n}";
        let old_string = "fn main() {\n    let x = 1;\n}";

        let matches = super::find_whitespace_normalized_matches(content, old_string);
        assert_eq!(matches.len(), 1, "Should find multi-line whitespace match");
    }

    #[test]
    fn test_normalize_whitespace() {
        let input = "fn   main()  {\n\tlet  x  =  1;\n}";
        let result = super::normalize_whitespace(input);
        assert_eq!(result, "fn main() { let x = 1; }");
    }

    // ================================================
    // PHASE 2: Fuzzy Matching Tests
    // ================================================

    #[test]
    fn test_fuzzy_find_whitespace_difference() {
        let content = "fn  main()  {";
        let old_string = "fn main() {";

        let result = super::fuzzy_find(content, old_string);
        assert!(result.is_some(), "Should find fuzzy match for whitespace difference");
    }

    #[test]
    fn test_fuzzy_find_rejects_different_code() {
        let content = "let result = self.validate();";
        let old_string = "let result = self.calculate();";

        let result = super::fuzzy_find(content, old_string);
        assert!(result.is_none(), "Should reject fuzzy match for different code");
    }

    #[test]
    fn test_fuzzy_find_uses_old_string_length() {
        let content = "fn  main()  {";
        let old_string = "fn main() {";

        let result = super::fuzzy_find(content, old_string);
        if let Some((_, len)) = result {
            assert_eq!(len, old_string.len(), "Should use old_string length for replacement");
        }
    }

    // ================================================
    // Phase 3: Edge Case Tests
    // ================================================

    #[test]
    fn test_empty_file() {
        let content = "";
        let matches = super::find_all_matches(content, "test");
        assert_eq!(matches.len(), 0, "Should not find matches in empty file");
    }

    #[test]
    fn test_edit_at_file_start() {
        let content = "abcdef";
        let matches = super::find_all_matches(content, "abc");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, 0, "Match should be at start");
    }

    #[test]
    fn test_edit_at_file_end() {
        let content = "abcdef";
        let matches = super::find_all_matches(content, "def");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, 3, "Match should be at end");
    }

    #[test]
    fn test_unicode_content() {
        let content = "héllo wørld 🌍";
        let matches = super::find_all_matches(content, "wørld");
        assert_eq!(matches.len(), 1, "Should find unicode match");
    }

    #[test]
    fn test_find_all_matches_multiple() {
        let content = "test one test two test three";
        let matches = super::find_all_matches(content, "test");
        assert_eq!(matches.len(), 3, "Should find all 3 matches");
    }

    #[test]
    fn test_find_all_matches_none() {
        let content = "hello world";
        let matches = super::find_all_matches(content, "xyz");
        assert_eq!(matches.len(), 0, "Should find no matches");
    }
}
