use std::path::{Path, PathBuf};

use forge_domain::{ContextSummary, SummaryMessage, SummaryTool, Transformer};

/// Strips the working directory prefix from all file paths in tool calls.
///
/// This transformer removes the working directory prefix from file paths in
/// FileRead, FileUpdate, and FileRemove tool calls, making paths relative to
/// the working directory. This is useful for reducing context size and making
/// summaries more portable across different environments.
///
/// # Platform-Specific Behavior
///
/// This implementation uses `std::path::Path::strip_prefix()`, which is
/// **platform-specific**:
///
/// - On **Windows**: Recognizes and strips Windows paths (e.g., `C:\Users\...`,
///   `\\server\share\...`)
/// - On **Unix/macOS**: Only recognizes Unix paths (forward slashes). Windows
///   paths are treated as literal strings and left unchanged.
///
/// This means:
/// - Windows paths in summaries will only be stripped when running on Windows
/// - Unix paths in summaries will only be stripped when running on Unix/macOS
/// - Cross-platform path handling would require a custom implementation that
///   doesn't rely on the OS-specific `std::path::Path`
///
/// For truly cross-platform path stripping (e.g., stripping Windows paths on
/// Unix or vice versa), consider implementing custom path parsing logic that
/// handles both path styles regardless of the host OS.
pub struct StripWorkingDir {
    working_dir: PathBuf,
}

impl StripWorkingDir {
    /// Creates a new StripWorkingDir transformer with the specified working
    /// directory.
    ///
    /// # Arguments
    ///
    /// * `working_dir` - The working directory path to strip from file paths
    pub fn new(working_dir: impl Into<PathBuf>) -> Self {
        Self { working_dir: working_dir.into() }
    }

    /// Strips the working directory prefix from a path if present.
    ///
    /// Returns the path with the working directory prefix removed, or the
    /// original path if it doesn't start with the working directory.
    fn strip_prefix(&self, path: &str) -> String {
        Path::new(path)
            .strip_prefix(&self.working_dir)
            .ok()
            .and_then(|p| p.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| path.to_string())
    }
}

impl Transformer for StripWorkingDir {
    type Value = ContextSummary;

    fn transform(&mut self, mut summary: Self::Value) -> Self::Value {
        for message in summary.messages.iter_mut() {
            for block in message.contents.iter_mut() {
                if let SummaryMessage::ToolCall(tool_data) = block {
                    match &mut tool_data.tool {
                        SummaryTool::FileRead { path } => {
                            *path = self.strip_prefix(path);
                        }
                        SummaryTool::FileUpdate { path } => {
                            *path = self.strip_prefix(path);
                        }
                        SummaryTool::FileRemove { path } => {
                            *path = self.strip_prefix(path);
                        }
                        SummaryTool::Undo { path } => {
                            *path = self.strip_prefix(path);
                        }
                        SummaryTool::Shell { .. }
                        | SummaryTool::Search { .. }
                        | SummaryTool::SemSearch { .. }
                        | SummaryTool::Fetch { .. }
                        | SummaryTool::Followup { .. }
                        | SummaryTool::Plan { .. }
                        | SummaryTool::Skill { .. }
                        | SummaryTool::Task { .. }
                        | SummaryTool::Mcp { .. }
                        | SummaryTool::TodoWrite { .. }
                        | SummaryTool::TodoRead => {
                            // These tools don't have paths to strip
                        }
                    }
                }
            }
        }

        summary
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Role, SummaryBlock, SummaryMessage as Block, SummaryToolCall};
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_empty_summary() {
        let fixture = ContextSummary::new(vec![]);
        let actual = StripWorkingDir::new("/home/user/project").transform(fixture);

        let expected = ContextSummary::new(vec![]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_strips_working_dir_from_file_read() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("/home/user/project/src/main.rs").into(),
                SummaryToolCall::read("/home/user/project/tests/test.rs").into(),
            ],
        )]);
        let actual = StripWorkingDir::new("/home/user/project").transform(fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("src/main.rs").into(),
                SummaryToolCall::read("tests/test.rs").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_strips_working_dir_from_file_update() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::update("/home/user/project/src/lib.rs").into(),
                SummaryToolCall::update("/home/user/project/README.md").into(),
            ],
        )]);
        let actual = StripWorkingDir::new("/home/user/project").transform(fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::update("src/lib.rs").into(),
                SummaryToolCall::update("README.md").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_strips_working_dir_from_file_remove() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::remove("/home/user/project/old.rs").into(),
                SummaryToolCall::remove("/home/user/project/deprecated/mod.rs").into(),
            ],
        )]);
        let actual = StripWorkingDir::new("/home/user/project").transform(fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::remove("old.rs").into(),
                SummaryToolCall::remove("deprecated/mod.rs").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_handles_paths_outside_working_dir() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("/home/user/project/src/main.rs").into(),
                SummaryToolCall::read("/etc/config.toml").into(),
                SummaryToolCall::update("/tmp/cache.json").into(),
            ],
        )]);
        let actual = StripWorkingDir::new("/home/user/project").transform(fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("src/main.rs").into(),
                SummaryToolCall::read("/etc/config.toml").into(),
                SummaryToolCall::update("/tmp/cache.json").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_handles_mixed_tool_calls() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("/home/user/project/src/main.rs").into(),
                SummaryToolCall::update("/home/user/project/src/lib.rs").into(),
                SummaryToolCall::remove("/home/user/project/old.rs").into(),
                SummaryToolCall::read("/other/path/file.rs").into(),
            ],
        )]);
        let actual = StripWorkingDir::new("/home/user/project").transform(fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("src/main.rs").into(),
                SummaryToolCall::update("src/lib.rs").into(),
                SummaryToolCall::remove("old.rs").into(),
                SummaryToolCall::read("/other/path/file.rs").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_handles_multiple_messages_and_roles() {
        let fixture = ContextSummary::new(vec![
            SummaryBlock::new(
                Role::User,
                vec![SummaryToolCall::read("/home/user/project/src/main.rs").into()],
            ),
            SummaryBlock::new(
                Role::Assistant,
                vec![
                    SummaryToolCall::read("/home/user/project/src/lib.rs").into(),
                    SummaryToolCall::update("/home/user/project/README.md").into(),
                ],
            ),
            SummaryBlock::new(
                Role::User,
                vec![SummaryToolCall::remove("/home/user/project/old.rs").into()],
            ),
        ]);
        let actual = StripWorkingDir::new("/home/user/project").transform(fixture);

        let expected = ContextSummary::new(vec![
            SummaryBlock::new(
                Role::User,
                vec![SummaryToolCall::read("src/main.rs").into()],
            ),
            SummaryBlock::new(
                Role::Assistant,
                vec![
                    SummaryToolCall::read("src/lib.rs").into(),
                    SummaryToolCall::update("README.md").into(),
                ],
            ),
            SummaryBlock::new(Role::User, vec![SummaryToolCall::remove("old.rs").into()]),
        ]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_preserves_blocks_without_tool_calls() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Some text content"),
                SummaryToolCall::read("/home/user/project/src/main.rs").into(),
                Block::content("More content"),
            ],
        )]);
        let actual = StripWorkingDir::new("/home/user/project").transform(fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Some text content"),
                SummaryToolCall::read("src/main.rs").into(),
                Block::content("More content"),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_handles_trailing_slash_in_working_dir() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![SummaryToolCall::read("/home/user/project/src/main.rs").into()],
        )]);
        let actual = StripWorkingDir::new("/home/user/project/").transform(fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![SummaryToolCall::read("src/main.rs").into()],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_handles_relative_paths() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("src/main.rs").into(),
                SummaryToolCall::update("./tests/test.rs").into(),
                SummaryToolCall::remove("../other/file.rs").into(),
            ],
        )]);
        let actual = StripWorkingDir::new("/home/user/project").transform(fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("src/main.rs").into(),
                SummaryToolCall::update("./tests/test.rs").into(),
                SummaryToolCall::remove("../other/file.rs").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_strips_windows_paths() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read(r"C:\Users\user\project\src\main.rs").into(),
                SummaryToolCall::update(r"C:\Users\user\project\tests\test.rs").into(),
            ],
        )]);
        let actual = StripWorkingDir::new(r"C:\Users\user\project").transform(fixture);

        // On Windows, paths are recognized and stripped
        #[cfg(windows)]
        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read(r"src\main.rs").into(),
                SummaryToolCall::update(r"tests\test.rs").into(),
            ],
        )]);

        // On Unix, Windows paths are not recognized, so they remain unchanged
        #[cfg(not(windows))]
        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read(r"C:\Users\user\project\src\main.rs").into(),
                SummaryToolCall::update(r"C:\Users\user\project\tests\test.rs").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_strips_windows_paths_with_forward_slashes() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("C:/Users/user/project/src/main.rs").into(),
                SummaryToolCall::update("C:/Users/user/project/tests/test.rs").into(),
            ],
        )]);
        let actual = StripWorkingDir::new("C:/Users/user/project").transform(fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("src/main.rs").into(),
                SummaryToolCall::update("tests/test.rs").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_strips_windows_paths_mixed_slashes() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read(r"C:\Users\user\project\src\main.rs").into(),
                SummaryToolCall::update("C:/Users/user/project/tests/test.rs").into(),
            ],
        )]);
        let actual = StripWorkingDir::new(r"C:\Users\user\project").transform(fixture);

        #[cfg(windows)]
        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read(r"src\main.rs").into(),
                SummaryToolCall::update("tests/test.rs").into(),
            ],
        )]);

        #[cfg(not(windows))]
        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read(r"C:\Users\user\project\src\main.rs").into(),
                SummaryToolCall::update("C:/Users/user/project/tests/test.rs").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_handles_windows_paths_outside_working_dir() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read(r"C:\Users\user\project\src\main.rs").into(),
                SummaryToolCall::read(r"D:\other\config.toml").into(),
                SummaryToolCall::update(r"C:\Windows\System32\file.dll").into(),
            ],
        )]);
        let actual = StripWorkingDir::new(r"C:\Users\user\project").transform(fixture);

        #[cfg(windows)]
        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read(r"src\main.rs").into(),
                SummaryToolCall::read(r"D:\other\config.toml").into(),
                SummaryToolCall::update(r"C:\Windows\System32\file.dll").into(),
            ],
        )]);

        #[cfg(not(windows))]
        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read(r"C:\Users\user\project\src\main.rs").into(),
                SummaryToolCall::read(r"D:\other\config.toml").into(),
                SummaryToolCall::update(r"C:\Windows\System32\file.dll").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_handles_windows_unc_paths() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read(r"\\server\share\project\src\main.rs").into(),
                SummaryToolCall::update(r"\\server\share\project\tests\test.rs").into(),
            ],
        )]);
        let actual = StripWorkingDir::new(r"\\server\share\project").transform(fixture);

        #[cfg(windows)]
        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read(r"src\main.rs").into(),
                SummaryToolCall::update(r"tests\test.rs").into(),
            ],
        )]);

        #[cfg(not(windows))]
        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read(r"\\server\share\project\src\main.rs").into(),
                SummaryToolCall::update(r"\\server\share\project\tests\test.rs").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_handles_windows_trailing_backslash() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![SummaryToolCall::read(r"C:\Users\user\project\src\main.rs").into()],
        )]);
        let actual = StripWorkingDir::new(r"C:\Users\user\project\").transform(fixture);

        #[cfg(windows)]
        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![SummaryToolCall::read(r"src\main.rs").into()],
        )]);

        #[cfg(not(windows))]
        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![SummaryToolCall::read(r"C:\Users\user\project\src\main.rs").into()],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_windows_case_sensitivity() {
        // On Windows, paths are case-insensitive, but we preserve the original case
        // when stripping. This test verifies case-sensitive matching behavior.
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read(r"C:\Users\User\Project\src\main.rs").into(),
                SummaryToolCall::update(r"c:\users\user\project\tests\test.rs").into(),
            ],
        )]);
        let actual = StripWorkingDir::new(r"C:\Users\User\Project").transform(fixture);

        // On Windows: case-insensitive matching, first path strips, second doesn't
        // On Unix: case-sensitive matching, neither path strips (Windows paths not
        // recognized)
        #[cfg(windows)]
        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read(r"src\main.rs").into(),
                SummaryToolCall::update(r"c:\users\user\project\tests\test.rs").into(),
            ],
        )]);

        #[cfg(not(windows))]
        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read(r"C:\Users\User\Project\src\main.rs").into(),
                SummaryToolCall::update(r"c:\users\user\project\tests\test.rs").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_windows_multiple_messages_and_roles() {
        let fixture = ContextSummary::new(vec![
            SummaryBlock::new(
                Role::User,
                vec![SummaryToolCall::read(r"C:\project\src\main.rs").into()],
            ),
            SummaryBlock::new(
                Role::Assistant,
                vec![
                    SummaryToolCall::read(r"C:\project\src\lib.rs").into(),
                    SummaryToolCall::update(r"C:\project\README.md").into(),
                ],
            ),
            SummaryBlock::new(
                Role::User,
                vec![SummaryToolCall::remove(r"C:\project\old.rs").into()],
            ),
        ]);
        let actual = StripWorkingDir::new(r"C:\project").transform(fixture);

        #[cfg(windows)]
        let expected = ContextSummary::new(vec![
            SummaryBlock::new(
                Role::User,
                vec![SummaryToolCall::read(r"src\main.rs").into()],
            ),
            SummaryBlock::new(
                Role::Assistant,
                vec![
                    SummaryToolCall::read(r"src\lib.rs").into(),
                    SummaryToolCall::update("README.md").into(),
                ],
            ),
            SummaryBlock::new(Role::User, vec![SummaryToolCall::remove("old.rs").into()]),
        ]);

        #[cfg(not(windows))]
        let expected = ContextSummary::new(vec![
            SummaryBlock::new(
                Role::User,
                vec![SummaryToolCall::read(r"C:\project\src\main.rs").into()],
            ),
            SummaryBlock::new(
                Role::Assistant,
                vec![
                    SummaryToolCall::read(r"C:\project\src\lib.rs").into(),
                    SummaryToolCall::update(r"C:\project\README.md").into(),
                ],
            ),
            SummaryBlock::new(
                Role::User,
                vec![SummaryToolCall::remove(r"C:\project\old.rs").into()],
            ),
        ]);

        assert_eq!(actual, expected);
    }
}
