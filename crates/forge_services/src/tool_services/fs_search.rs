use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, anyhow};
use bstr::ByteSlice;
use forge_app::{
    FileInfoInfra, FileReaderInfra, FsSearchService, Match, MatchResult, SearchResult, Walker,
    WalkerInfra,
};
use forge_domain::{FSSearch, OutputMode};
use grep_regex::RegexMatcherBuilder;
use grep_searcher::sinks::UTF8;
use grep_searcher::{Searcher, SearcherBuilder, Sink, SinkContext, SinkContextKind, SinkMatch};

/// A powerful search tool built on grep-matcher and grep-searcher crates.
/// Supports regex patterns, file type filtering, output modes, context lines,
/// and multiline matching.
pub struct ForgeFsSearch<W> {
    infra: Arc<W>,
}

impl<W> ForgeFsSearch<W> {
    pub fn new(infra: Arc<W>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<W: WalkerInfra + FileReaderInfra + FileInfoInfra> FsSearchService for ForgeFsSearch<W> {
    async fn search(&self, params: FSSearch) -> anyhow::Result<Option<SearchResult>> {
        // Determine search path (default to current directory)
        let search_path = match &params.path {
            Some(p) if !p.is_empty() => PathBuf::from(p),
            _ => std::env::current_dir()
                .with_context(|| "Failed to get current working directory")?,
        };

        // Validate path exists
        if !self.infra.exists(&search_path).await? {
            return Err(anyhow!("Path does not exist: {}", search_path.display()));
        }

        // Build regex matcher
        let matcher = self.build_matcher(&params)?;

        // Get file paths to search
        let file_paths = self.get_matching_files(&search_path, &params).await?;

        if file_paths.is_empty() {
            return Ok(None);
        }

        // Determine output mode (default to FilesWithMatches)
        let output_mode = params
            .output_mode
            .as_ref()
            .unwrap_or(&OutputMode::FilesWithMatches);

        // Execute search based on output mode
        let matches = match output_mode {
            OutputMode::FilesWithMatches => {
                self.search_files_with_matches(&file_paths, &matcher)
                    .await?
            }
            OutputMode::Content => self.search_content(&file_paths, &matcher, &params).await?,
            OutputMode::Count => self.search_count(&file_paths, &matcher).await?,
        };

        if matches.is_empty() {
            Ok(None)
        } else {
            Ok(Some(SearchResult { matches }))
        }
    }
}

impl<W: WalkerInfra + FileReaderInfra + FileInfoInfra> ForgeFsSearch<W> {
    /// Builds a regex matcher from search parameters
    fn build_matcher(&self, params: &FSSearch) -> anyhow::Result<grep_regex::RegexMatcher> {
        let mut builder = RegexMatcherBuilder::new();

        // Apply case insensitivity (default to case-sensitive)
        if params.case_insensitive.unwrap_or(false) {
            builder.case_insensitive(true);
        }

        // Apply multiline mode
        if params.multiline.unwrap_or(false) {
            builder.multi_line(true);
            builder.dot_matches_new_line(true);
        }

        builder
            .build(&params.pattern)
            .with_context(|| format!("Invalid regex pattern: {}", params.pattern))
    }

    /// Gets list of files to search based on glob and file type filters
    async fn get_matching_files(
        &self,
        search_path: &Path,
        params: &FSSearch,
    ) -> anyhow::Result<Vec<PathBuf>> {
        // Build type matcher once if file_type is specified (for efficiency)
        // Filter out empty strings that may arrive from LLM tool calls with nullable
        // parameters
        let types_matcher =
            if let Some(file_type) = params.file_type.as_deref().filter(|s| !s.is_empty()) {
                use ignore::types::TypesBuilder;

                let mut builder = TypesBuilder::new();
                builder.add_defaults();
                builder.select(file_type);

                Some(
                    builder.build().with_context(|| {
                        format!("Failed to build type matcher for: {file_type}")
                    })?,
                )
            } else {
                None
            };

        let paths = if self.infra.is_file(search_path).await? {
            vec![search_path.to_path_buf()]
        } else {
            self.walk_directory(search_path).await?
        };

        // Apply file filtering
        let mut filtered_paths = Vec::new();
        for path in paths {
            if self
                .matches_file_filters(&path, params, types_matcher.as_ref())
                .await?
            {
                filtered_paths.push(path);
            }
        }

        Ok(filtered_paths)
    }

    /// Walks a directory to get all files
    async fn walk_directory(&self, dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
        let walked_files = self
            .infra
            .walk(Walker::unlimited().cwd(dir.to_path_buf()))
            .await
            .with_context(|| format!("Failed to walk directory '{}'", dir.display()))?;

        let mut paths = Vec::new();
        for file in walked_files {
            let path = dir.join(file.path);
            if self.infra.is_file(&path).await? {
                paths.push(path);
            }
        }

        Ok(paths)
    }

    /// Checks if a file matches the glob and type filters
    async fn matches_file_filters(
        &self,
        path: &Path,
        params: &FSSearch,
        types_matcher: Option<&ignore::types::Types>,
    ) -> anyhow::Result<bool> {
        // Must be a file
        if !self.infra.is_file(path).await? {
            return Ok(false);
        }

        // Apply glob filter if provided
        if let Some(glob_pattern) = &params.glob {
            let pattern = glob::Pattern::new(glob_pattern)
                .with_context(|| format!("Invalid glob pattern: {glob_pattern}"))?;

            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                if !pattern.matches(file_name) {
                    return Ok(false);
                }
            } else {
                return Ok(false);
            }
        }

        // Apply file type filter if provided (only if glob not specified)
        if params.glob.is_none()
            && let Some(types) = types_matcher
        {
            return self.matches_file_type(path, types);
        }

        Ok(true)
    }

    /// Checks if a file matches a given file type using ignore crate's type
    /// definitions
    fn matches_file_type(&self, path: &Path, types: &ignore::types::Types) -> anyhow::Result<bool> {
        // Check if the file matches the type
        let matched = types.matched(path, false).is_whitelist();

        Ok(matched)
    }

    /// Searches files and returns only paths that have matches
    async fn search_files_with_matches(
        &self,
        paths: &[PathBuf],
        matcher: &grep_regex::RegexMatcher,
    ) -> anyhow::Result<Vec<Match>> {
        let mut matches = Vec::new();

        for path in paths {
            // Skip binary files
            if self.infra.is_binary(path).await? {
                continue;
            }

            let content = self.infra.read(path).await?;

            // Check if file has any matches
            let mut has_match = false;
            Searcher::new().search_slice(
                matcher,
                &content,
                UTF8(|_, _| {
                    // Found a match, set flag and stop searching
                    has_match = true;
                    Ok(false)
                }),
            )?;

            if has_match {
                matches.push(Match {
                    path: path.to_string_lossy().to_string(),
                    result: Some(MatchResult::FileMatch),
                });
            }
        }

        Ok(matches)
    }

    /// Searches files and returns match counts per file
    async fn search_count(
        &self,
        paths: &[PathBuf],
        matcher: &grep_regex::RegexMatcher,
    ) -> anyhow::Result<Vec<Match>> {
        let mut matches = Vec::new();

        for path in paths {
            // Skip binary files
            if self.infra.is_binary(path).await? {
                continue;
            }

            let content = self.infra.read(path).await?;
            let mut count = 0usize;

            Searcher::new().search_slice(
                matcher,
                &content,
                UTF8(|_, _| {
                    count += 1;
                    Ok(true)
                }),
            )?;

            if count > 0 {
                matches.push(Match {
                    path: path.to_string_lossy().to_string(),
                    result: Some(MatchResult::Count { count }),
                });
            }
        }

        Ok(matches)
    }
}

/// Custom sink for capturing matches with context lines.
///
/// This sink implements the full `Sink` trait from grep-searcher to capture
/// both matches and their surrounding context lines. The grep-searcher's
/// convenience sinks (UTF8, Lossy, Bytes) only report matches and ignore
/// context, so we need a custom implementation.
///
/// # Context Handling
///
/// - **Before context**: Lines that appear before a match are accumulated in
///   `before_context` as they arrive via `context()` calls with
///   `SinkContextKind::Before`.
/// - **Match**: When a match is found via `matched()`, we save any pending
///   match (from the previous iteration) along with its contexts, then store
///   the new match.
/// - **After context**: Lines that appear after a match are accumulated in
///   `current_after_context` as they arrive via `context()` calls with
///   `SinkContextKind::After`.
/// - **Final flush**: When the search completes, `into_matches()` is called to
///   flush the last pending match with its accumulated contexts.
struct ContextSink {
    path: String,
    show_line_numbers: bool,
    matches: Vec<Match>,
    before_context: Vec<String>,
    current_match: Option<(usize, String)>,
    current_after_context: Vec<String>,
}

impl ContextSink {
    fn new(path: String, show_line_numbers: bool) -> Self {
        Self {
            path,
            show_line_numbers,
            matches: Vec::new(),
            before_context: Vec::new(),
            current_match: None,
            current_after_context: Vec::new(),
        }
    }

    fn into_matches(mut self, has_context: bool) -> Vec<Match> {
        // If we have a pending match, flush it
        if let Some((line_num, line)) = self.current_match.take() {
            if has_context {
                self.matches.push(Match {
                    path: self.path.clone(),
                    result: Some(MatchResult::ContextMatch {
                        line_number: if self.show_line_numbers {
                            Some(line_num)
                        } else {
                            None
                        },
                        line,
                        before_context: self.before_context.clone(),
                        after_context: self.current_after_context.clone(),
                    }),
                });
            } else {
                self.matches.push(Match {
                    path: self.path.clone(),
                    result: Some(MatchResult::Found {
                        line_number: if self.show_line_numbers {
                            Some(line_num)
                        } else {
                            None
                        },
                        line,
                    }),
                });
            }
        }
        self.matches
    }
}

impl Sink for ContextSink {
    type Error = std::io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        // If we have a pending match, save it first (it's now complete with all after
        // context)
        if let Some((line_num, line)) = self.current_match.take() {
            self.matches.push(Match {
                path: self.path.clone(),
                result: Some(MatchResult::ContextMatch {
                    line_number: if self.show_line_numbers {
                        Some(line_num)
                    } else {
                        None
                    },
                    line,
                    before_context: self.before_context.clone(),
                    after_context: self.current_after_context.clone(),
                }),
            });

            // Clear contexts now that we've used them
            self.before_context.clear();
            self.current_after_context.clear();
        }

        // Store the current match (before_context is already accumulated, after_context
        // will be added via context() calls)
        let line_num = mat.line_number().unwrap_or(0) as usize;
        let line = mat.bytes().to_str_lossy().trim_end().to_string();
        self.current_match = Some((line_num, line));

        Ok(true)
    }

    fn context(
        &mut self,
        _searcher: &Searcher,
        ctx: &SinkContext<'_>,
    ) -> Result<bool, Self::Error> {
        let line = ctx.bytes().to_str_lossy().trim_end().to_string();

        match ctx.kind() {
            SinkContextKind::Before => {
                // Accumulate before context for the next match
                self.before_context.push(line);
            }
            SinkContextKind::After => {
                // Add to the current match's after_context
                self.current_after_context.push(line);
            }
            _ => {}
        }

        Ok(true)
    }
}

impl<W: WalkerInfra + FileReaderInfra + FileInfoInfra> ForgeFsSearch<W> {
    /// Searches files and returns matching lines with content
    async fn search_content(
        &self,
        paths: &[PathBuf],
        matcher: &grep_regex::RegexMatcher,
        params: &FSSearch,
    ) -> anyhow::Result<Vec<Match>> {
        let show_line_numbers = params.show_line_numbers.unwrap_or(true);

        // Check if context lines are requested
        let has_context = params.context.is_some()
            || params.before_context.is_some()
            || params.after_context.is_some();

        // Configure searcher with context lines
        let mut searcher_builder = SearcherBuilder::new();
        searcher_builder.line_number(true); // Always enable line numbers for the searcher

        // Determine context lines
        if let Some(context) = params.context {
            searcher_builder.before_context(context as usize);
            searcher_builder.after_context(context as usize);
        } else {
            if let Some(before) = params.before_context {
                searcher_builder.before_context(before as usize);
            }
            if let Some(after) = params.after_context {
                searcher_builder.after_context(after as usize);
            }
        }

        let mut searcher = searcher_builder.build();
        let mut all_matches = Vec::new();

        for path in paths {
            // Skip binary files
            if self.infra.is_binary(path).await? {
                continue;
            }

            let content = self.infra.read(path).await?;
            let path_string = path.to_string_lossy().to_string();

            if has_context {
                // Use custom sink to capture context lines
                let mut sink = ContextSink::new(path_string.clone(), show_line_numbers);
                searcher.search_slice(matcher, &content, &mut sink)?;
                all_matches.extend(sink.into_matches(true));
            } else {
                // Use simple UTF8 sink for matches without context
                searcher.search_slice(
                    matcher,
                    &content,
                    UTF8(|line_num, line| {
                        all_matches.push(Match {
                            path: path_string.clone(),
                            result: Some(MatchResult::Found {
                                line_number: if show_line_numbers {
                                    Some(line_num as usize)
                                } else {
                                    None
                                },
                                line: line.trim_end().to_string(),
                            }),
                        });
                        Ok(true)
                    }),
                )?;
            }
        }

        Ok(all_matches)
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashSet;
    use std::sync::Arc;

    use forge_app::{WalkedFile, Walker};
    use forge_domain::{FSSearch, OutputMode};
    use tokio::fs;

    use super::*;
    use crate::utils::TempDir;

    // Mock infrastructure for testing
    struct MockInfra {
        binary_exts: HashSet<String>,
    }

    impl Default for MockInfra {
        fn default() -> Self {
            let binary_exts = [
                "exe", "dll", "so", "dylib", "bin", "obj", "o", "class", "pyc", "jar", "war",
                "ear", "zip", "tar", "gz", "rar", "7z", "iso", "img", "pdf", "doc", "docx", "xls",
                "xlsx", "ppt", "pptx", "bmp", "ico", "mp3", "mp4", "avi", "mov", "sqlite", "db",
            ];
            Self {
                binary_exts: HashSet::from_iter(binary_exts.into_iter().map(|ext| ext.to_string())),
            }
        }
    }

    #[async_trait::async_trait]
    impl FileReaderInfra for MockInfra {
        async fn read_utf8(&self, _path: &Path) -> anyhow::Result<String> {
            unimplemented!()
        }

        fn read_batch_utf8(
            &self,
            _batch_size: usize,
            _paths: Vec<PathBuf>,
        ) -> impl futures::Stream<Item = (PathBuf, anyhow::Result<String>)> + Send {
            futures::stream::empty()
        }

        async fn read(&self, path: &Path) -> anyhow::Result<Vec<u8>> {
            fs::read(path)
                .await
                .with_context(|| format!("Failed to read file '{}'", path.display()))
        }

        async fn range_read_utf8(
            &self,
            _path: &Path,
            _start_line: u64,
            _end_line: u64,
        ) -> anyhow::Result<(String, forge_domain::FileInfo)> {
            unimplemented!()
        }
    }

    #[async_trait::async_trait]
    impl FileInfoInfra for MockInfra {
        async fn is_file(&self, path: &Path) -> anyhow::Result<bool> {
            let metadata = tokio::fs::metadata(path).await;
            match metadata {
                Ok(meta) => Ok(meta.is_file()),
                Err(_) => Ok(false),
            }
        }

        async fn is_binary(&self, path: &Path) -> anyhow::Result<bool> {
            let ext = path.extension().and_then(|s| s.to_str());
            Ok(self.binary_exts.contains(ext.unwrap_or("")))
        }

        async fn exists(&self, path: &Path) -> anyhow::Result<bool> {
            Ok(tokio::fs::metadata(path).await.is_ok())
        }

        async fn file_size(&self, _path: &Path) -> anyhow::Result<u64> {
            unreachable!()
        }
    }

    #[async_trait::async_trait]
    impl WalkerInfra for MockInfra {
        async fn walk(&self, config: Walker) -> anyhow::Result<Vec<WalkedFile>> {
            let mut files = Vec::new();
            let metadata = tokio::fs::metadata(&config.cwd).await?;
            if metadata.is_dir() {
                let mut entries = tokio::fs::read_dir(&config.cwd).await?;
                while let Some(entry) = entries.next_entry().await? {
                    let path = entry.path();
                    if path.is_file() {
                        let relative_path = path
                            .strip_prefix(&config.cwd)?
                            .to_string_lossy()
                            .to_string();
                        let file_name = path.file_name().map(|n| n.to_string_lossy().to_string());
                        let size = entry.metadata().await?.len();

                        files.push(WalkedFile { path: relative_path, file_name, size });
                    }
                }
            }
            Ok(files)
        }
    }

    async fn create_test_directory() -> anyhow::Result<TempDir> {
        let temp_dir = TempDir::new()?;

        fs::write(
            temp_dir.path().join("test.txt"),
            "hello world\ntest line\nfoo bar",
        )
        .await?;
        fs::write(temp_dir.path().join("other.txt"), "no match here").await?;
        fs::write(
            temp_dir.path().join("code.rs"),
            "fn test() {}\nfn main() {}",
        )
        .await?;
        fs::write(temp_dir.path().join("app.js"), "function test() {}").await?;

        Ok(temp_dir)
    }

    #[tokio::test]
    async fn test_basic_content_search() {
        let fixture = create_test_directory().await.unwrap();
        let params = FSSearch {
            pattern: "test".to_string(),
            path: Some(fixture.path().to_string_lossy().to_string()),
            output_mode: Some(OutputMode::Content),
            ..Default::default()
        };

        let actual = ForgeFsSearch::new(Arc::new(MockInfra::default()))
            .search(params)
            .await
            .unwrap();

        assert!(actual.is_some());
        let result = actual.unwrap();
        assert!(!result.matches.is_empty());
        // Should find matches in test.txt, code.rs, and app.js
        assert!(result.matches.len() >= 3);
    }

    #[tokio::test]
    async fn test_files_with_matches_mode() {
        let fixture = create_test_directory().await.unwrap();
        let params = FSSearch {
            pattern: "test".to_string(),
            path: Some(fixture.path().to_string_lossy().to_string()),
            output_mode: Some(OutputMode::FilesWithMatches),
            ..Default::default()
        };

        let actual = ForgeFsSearch::new(Arc::new(MockInfra::default()))
            .search(params)
            .await
            .unwrap();

        assert!(actual.is_some());
        let result = actual.unwrap();
        // Should return file paths only, no content
        assert!(
            result
                .matches
                .iter()
                .all(|m| m.result.is_some() && matches!(m.result, Some(MatchResult::FileMatch)))
        );
    }

    #[tokio::test]
    async fn test_count_mode() {
        let fixture = create_test_directory().await.unwrap();
        let params = FSSearch {
            pattern: "test".to_string(),
            path: Some(fixture.path().to_string_lossy().to_string()),
            output_mode: Some(OutputMode::Count),
            ..Default::default()
        };

        let actual = ForgeFsSearch::new(Arc::new(MockInfra::default()))
            .search(params)
            .await
            .unwrap();

        assert!(actual.is_some());
        let result = actual.unwrap();
        // Should return counts
        assert!(
            result
                .matches
                .iter()
                .all(|m| matches!(m.result, Some(MatchResult::Count { count: _ })))
        );
    }

    #[tokio::test]
    async fn test_empty_file_type_treated_as_none() {
        let fixture = create_test_directory().await.unwrap();
        let params = FSSearch {
            pattern: "test".to_string(),
            path: Some(fixture.path().to_string_lossy().to_string()),
            file_type: Some("".to_string()),
            output_mode: Some(OutputMode::FilesWithMatches),
            ..Default::default()
        };

        // Should not error - empty file_type should be treated as None
        let actual = ForgeFsSearch::new(Arc::new(MockInfra::default()))
            .search(params)
            .await
            .unwrap();

        assert!(actual.is_some());
        let result = actual.unwrap();
        // Should match files across all types (not filtered)
        assert!(result.matches.len() >= 3);
    }

    #[tokio::test]
    async fn test_glob_filter() {
        let fixture = create_test_directory().await.unwrap();
        let params = FSSearch {
            pattern: "test".to_string(),
            path: Some(fixture.path().to_string_lossy().to_string()),
            glob: Some("*.rs".to_string()),
            output_mode: Some(OutputMode::FilesWithMatches),
            ..Default::default()
        };

        let actual = ForgeFsSearch::new(Arc::new(MockInfra::default()))
            .search(params)
            .await
            .unwrap();

        assert!(actual.is_some());
        let result = actual.unwrap();
        // Should only match .rs files
        assert!(result.matches.iter().all(|m| m.path.ends_with(".rs")));
    }

    #[tokio::test]
    async fn test_case_insensitive() {
        let fixture = create_test_directory().await.unwrap();
        let params = FSSearch {
            pattern: "HELLO".to_string(),
            path: Some(fixture.path().to_string_lossy().to_string()),
            case_insensitive: Some(true),
            output_mode: Some(OutputMode::Content),
            ..Default::default()
        };

        let actual = ForgeFsSearch::new(Arc::new(MockInfra::default()))
            .search(params)
            .await
            .unwrap();

        assert!(actual.is_some());
    }

    #[tokio::test]
    async fn test_case_sensitive_default() {
        let fixture = create_test_directory().await.unwrap();
        let params = FSSearch {
            pattern: "HELLO".to_string(),
            path: Some(fixture.path().to_string_lossy().to_string()),
            output_mode: Some(OutputMode::Content),
            ..Default::default()
        };

        let actual = ForgeFsSearch::new(Arc::new(MockInfra::default()))
            .search(params)
            .await
            .unwrap();

        // Should not match because it's case-sensitive by default
        assert!(actual.is_none());
    }

    #[tokio::test]
    async fn test_line_numbers_enabled() {
        let fixture = create_test_directory().await.unwrap();
        let params = FSSearch {
            pattern: "test".to_string(),
            path: Some(fixture.path().to_string_lossy().to_string()),
            output_mode: Some(OutputMode::Content),
            show_line_numbers: Some(true),
            ..Default::default()
        };

        let actual = ForgeFsSearch::new(Arc::new(MockInfra::default()))
            .search(params)
            .await
            .unwrap();

        assert!(actual.is_some());
        let result = actual.unwrap();
        // Should have line numbers
        assert!(
            result
                .matches
                .iter()
                .filter_map(|m| m.result.as_ref())
                .all(|r| matches!(r, MatchResult::Found { line_number: Some(_), .. }))
        );
    }

    #[tokio::test]
    async fn test_line_numbers_disabled() {
        let fixture = create_test_directory().await.unwrap();
        let params = FSSearch {
            pattern: "test".to_string(),
            path: Some(fixture.path().to_string_lossy().to_string()),
            output_mode: Some(OutputMode::Content),
            show_line_numbers: Some(false),
            ..Default::default()
        };

        let actual = ForgeFsSearch::new(Arc::new(MockInfra::default()))
            .search(params)
            .await
            .unwrap();

        assert!(actual.is_some());
        let result = actual.unwrap();
        // Should not have line numbers
        assert!(
            result
                .matches
                .iter()
                .filter_map(|m| m.result.as_ref())
                .all(|r| matches!(r, MatchResult::Found { line_number: None, .. }))
        );
    }

    #[tokio::test]
    async fn test_path_defaults_to_cwd() {
        let params = FSSearch {
            pattern: "test".to_string(),
            path: None,
            ..Default::default()
        };

        // This should use current directory
        let result = ForgeFsSearch::new(Arc::new(MockInfra::default()))
            .search(params)
            .await;

        // Should not error (even if no matches found)
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_no_matches_returns_none() {
        let fixture = create_test_directory().await.unwrap();
        let params = FSSearch {
            pattern: "nonexistent_pattern_xyz".to_string(),
            path: Some(fixture.path().to_string_lossy().to_string()),
            ..Default::default()
        };

        let actual = ForgeFsSearch::new(Arc::new(MockInfra::default()))
            .search(params)
            .await
            .unwrap();

        assert!(actual.is_none());
    }

    #[tokio::test]
    async fn test_skip_binary_files() {
        let fixture = TempDir::new().unwrap();
        fs::write(fixture.path().join("text.txt"), "hello world")
            .await
            .unwrap();
        fs::write(fixture.path().join("binary.exe"), "hello world")
            .await
            .unwrap();

        let params = FSSearch {
            pattern: "hello".to_string(),
            path: Some(fixture.path().to_string_lossy().to_string()),
            output_mode: Some(OutputMode::FilesWithMatches),
            ..Default::default()
        };

        let actual = ForgeFsSearch::new(Arc::new(MockInfra::default()))
            .search(params)
            .await
            .unwrap();

        assert!(actual.is_some());
        let result = actual.unwrap();
        // Should only find text.txt, not binary.exe
        assert_eq!(result.matches.len(), 1);
        assert!(result.matches[0].path.ends_with("text.txt"));
    }

    #[tokio::test]
    async fn test_context_lines_both() {
        let fixture = TempDir::new().unwrap();
        fs::write(
            fixture.path().join("test.txt"),
            "line 1\nline 2\nline 3\nMATCH HERE\nline 5\nline 6\nline 7",
        )
        .await
        .unwrap();

        let params = FSSearch {
            pattern: "MATCH".to_string(),
            path: Some(fixture.path().to_string_lossy().to_string()),
            output_mode: Some(OutputMode::Content),
            context: Some(2), // 2 lines before and after
            ..Default::default()
        };

        let actual = ForgeFsSearch::new(Arc::new(MockInfra::default()))
            .search(params)
            .await
            .unwrap();

        assert!(actual.is_some());
        let result = actual.unwrap();
        assert_eq!(result.matches.len(), 1);

        // Verify it's a ContextMatch with before and after context
        match &result.matches[0].result {
            Some(MatchResult::ContextMatch {
                line,
                before_context,
                after_context,
                line_number,
            }) => {
                assert_eq!(line, "MATCH HERE");
                assert_eq!(line_number, &Some(4));
                assert_eq!(before_context.len(), 2);
                assert_eq!(before_context[0], "line 2");
                assert_eq!(before_context[1], "line 3");
                assert_eq!(after_context.len(), 2);
                assert_eq!(after_context[0], "line 5");
                assert_eq!(after_context[1], "line 6");
            }
            _ => panic!("Expected ContextMatch, got {:?}", result.matches[0].result),
        }
    }

    #[tokio::test]
    async fn test_before_context_only() {
        let fixture = TempDir::new().unwrap();
        fs::write(
            fixture.path().join("test.txt"),
            "line 1\nline 2\nline 3\nMATCH HERE\nline 5\nline 6",
        )
        .await
        .unwrap();

        let params = FSSearch {
            pattern: "MATCH".to_string(),
            path: Some(fixture.path().to_string_lossy().to_string()),
            output_mode: Some(OutputMode::Content),
            before_context: Some(2), // 2 lines before only
            ..Default::default()
        };

        let actual = ForgeFsSearch::new(Arc::new(MockInfra::default()))
            .search(params)
            .await
            .unwrap();

        assert!(actual.is_some());
        let result = actual.unwrap();
        assert_eq!(result.matches.len(), 1);

        match &result.matches[0].result {
            Some(MatchResult::ContextMatch { line, before_context, after_context, .. }) => {
                assert_eq!(line, "MATCH HERE");
                assert_eq!(before_context.len(), 2);
                assert_eq!(before_context[0], "line 2");
                assert_eq!(before_context[1], "line 3");
                assert_eq!(after_context.len(), 0); // No after context
            }
            _ => panic!("Expected ContextMatch"),
        }
    }

    #[tokio::test]
    async fn test_after_context_only() {
        let fixture = TempDir::new().unwrap();
        fs::write(
            fixture.path().join("test.txt"),
            "line 1\nline 2\nMATCH HERE\nline 4\nline 5\nline 6",
        )
        .await
        .unwrap();

        let params = FSSearch {
            pattern: "MATCH".to_string(),
            path: Some(fixture.path().to_string_lossy().to_string()),
            output_mode: Some(OutputMode::Content),
            after_context: Some(2), // 2 lines after only
            ..Default::default()
        };

        let actual = ForgeFsSearch::new(Arc::new(MockInfra::default()))
            .search(params)
            .await
            .unwrap();

        assert!(actual.is_some());
        let result = actual.unwrap();
        assert_eq!(result.matches.len(), 1);

        match &result.matches[0].result {
            Some(MatchResult::ContextMatch { line, before_context, after_context, .. }) => {
                assert_eq!(line, "MATCH HERE");
                assert_eq!(before_context.len(), 0); // No before context
                assert_eq!(after_context.len(), 2);
                assert_eq!(after_context[0], "line 4");
                assert_eq!(after_context[1], "line 5");
            }
            _ => panic!("Expected ContextMatch"),
        }
    }

    #[tokio::test]
    async fn test_no_context_returns_found() {
        let fixture = TempDir::new().unwrap();
        fs::write(
            fixture.path().join("test.txt"),
            "line 1\nMATCH HERE\nline 3",
        )
        .await
        .unwrap();

        let params = FSSearch {
            pattern: "MATCH".to_string(),
            path: Some(fixture.path().to_string_lossy().to_string()),
            output_mode: Some(OutputMode::Content),
            // No context specified
            ..Default::default()
        };

        let actual = ForgeFsSearch::new(Arc::new(MockInfra::default()))
            .search(params)
            .await
            .unwrap();

        assert!(actual.is_some());
        let result = actual.unwrap();
        assert_eq!(result.matches.len(), 1);

        // Should be Found, not ContextMatch when no context is requested
        match &result.matches[0].result {
            Some(MatchResult::Found { line, .. }) => {
                assert_eq!(line, "MATCH HERE");
            }
            _ => panic!("Expected Found, got {:?}", result.matches[0].result),
        }
    }
}
