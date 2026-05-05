use forge_display::DiffFormat;
use forge_domain::{ChatResponseContent, Environment, TitleFormat};

use crate::fmt::content::FormatContent;
use crate::fmt::todo_fmt::{format_todos, format_todos_diff};
use crate::operation::ToolOperation;
use crate::utils::format_display_path;

impl FormatContent for ToolOperation {
    fn to_content(&self, env: &Environment) -> Option<ChatResponseContent> {
        match self {
            ToolOperation::FsWrite { input, output } => {
                if let Some(ref before) = output.before {
                    let after = &input.content;
                    Some(ChatResponseContent::ToolOutput(
                        DiffFormat::format(before, after).diff().to_string(),
                    ))
                } else {
                    None
                }
            }
            ToolOperation::FsPatch { input: _, output } => Some(ChatResponseContent::ToolOutput(
                DiffFormat::format(&output.before, &output.after)
                    .diff()
                    .to_string(),
            )),
            ToolOperation::FsMultiPatch { input: _, output } => {
                Some(ChatResponseContent::ToolOutput(
                    DiffFormat::format(&output.before, &output.after)
                        .diff()
                        .to_string(),
                ))
            }
            ToolOperation::PlanCreate { input: _, output } => Some({
                let title = TitleFormat::debug(format!(
                    "Create {}",
                    format_display_path(&output.path, &env.cwd)
                ));
                title.into()
            }),
            ToolOperation::TodoWrite { before, after } => Some(ChatResponseContent::ToolOutput(
                format_todos_diff(before, after),
            )),
            ToolOperation::TodoRead { output } => {
                Some(ChatResponseContent::ToolOutput(format_todos(output)))
            }
            ToolOperation::FsRead { input: _, output: _ }
            | ToolOperation::FsRemove { input: _, output: _ }
            | ToolOperation::FsSearch { input: _, output: _ }
            | ToolOperation::CodebaseSearch { output: _ }
            | ToolOperation::FsUndo { input: _, output: _ }
            | ToolOperation::NetFetch { input: _, output: _ }
            | ToolOperation::Shell { output: _ }
            | ToolOperation::FollowUp { output: _ }
            | ToolOperation::Skill { output: _ } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use console::strip_ansi_codes;
    use forge_display::DiffFormat;
    use forge_domain::{ChatResponseContent, Environment, FileInfo};
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;

    use super::FormatContent;
    // ContentFormat is now ChatResponseContent
    use crate::operation::ToolOperation;
    use crate::{
        Content, FsRemoveOutput, FsUndoOutput, FsWriteOutput, HttpResponse, Match, MatchResult,
        PatchOutput, ReadOutput, ResponseContext, SearchResult, ShellOutput,
    };

    // ContentFormat methods are now implemented in ChatResponseContent

    fn fixture_environment() -> Environment {
        use fake::{Fake, Faker};
        Faker.fake()
    }

    #[test]
    fn test_fs_read_single_line() {
        let content = "Hello, world!";
        let fixture = ToolOperation::FsRead {
            input: forge_domain::FSRead {
                file_path: "/home/user/test.txt".to_string(),
                range: None,
                show_line_numbers: true,
            },
            output: ReadOutput {
                content: Content::file(content),
                info: FileInfo::new(1, 1, 5, crate::compute_hash(content)),
            },
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_fs_read_multiple_lines() {
        let content = "Line 1\nLine 2\nLine 3";
        let fixture = ToolOperation::FsRead {
            input: forge_domain::FSRead {
                file_path: "/home/user/test.txt".to_string(),
                range: Some(forge_domain::FSReadRange { start_line: Some(2), end_line: Some(4) }),
                show_line_numbers: true,
            },
            output: ReadOutput {
                content: Content::file(content),
                info: FileInfo::new(2, 4, 10, crate::compute_hash(content)),
            },
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_fs_create_new_file() {
        let content = "New file content";
        let fixture = ToolOperation::FsWrite {
            input: forge_domain::FSWrite {
                file_path: "/home/user/project/new_file.txt".to_string(),
                content: content.to_string(),
                overwrite: false,
            },
            output: FsWriteOutput {
                path: "/home/user/project/new_file.txt".to_string(),
                before: None,
                errors: vec![],
                content_hash: crate::compute_hash(content),
            },
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_fs_create_overwrite() {
        let content = "new content";
        let fixture = ToolOperation::FsWrite {
            input: forge_domain::FSWrite {
                file_path: "/home/user/project/existing_file.txt".to_string(),
                content: content.to_string(),
                overwrite: true,
            },
            output: FsWriteOutput {
                path: "/home/user/project/existing_file.txt".to_string(),
                before: Some("old content".to_string()),
                errors: vec![],
                content_hash: crate::compute_hash(content),
            },
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = Some(ChatResponseContent::ToolOutput(
            DiffFormat::format("old content", "new content")
                .diff()
                .to_string(),
        ));

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_fs_create_with_warning() {
        let content = "File content";
        let fixture = ToolOperation::FsWrite {
            input: forge_domain::FSWrite {
                file_path: "/home/user/project/file.txt".to_string(),
                content: content.to_string(),
                overwrite: false,
            },
            output: FsWriteOutput {
                path: "/home/user/project/file.txt".to_string(),
                before: None,
                errors: vec![forge_domain::SyntaxError {
                    line: 5,
                    column: 10,
                    message: "Syntax error".to_string(),
                }],
                content_hash: crate::compute_hash(content),
            },
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_fs_remove() {
        let fixture = ToolOperation::FsRemove {
            input: forge_domain::FSRemove { path: "/home/user/project/file.txt".to_string() },
            output: FsRemoveOutput { content: "".to_string() },
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_fs_search_with_matches() {
        let fixture = ToolOperation::FsSearch {
            input: forge_domain::FSSearch {
                path: Some("/home/user/project".to_string()),
                pattern: "Hello".to_string(),
                ..Default::default()
            },
            output: Some(SearchResult {
                matches: vec![
                    Match {
                        path: "file1.txt".to_string(),
                        result: Some(MatchResult::Found {
                            line_number: Some(1),
                            line: "Hello world".to_string(),
                        }),
                    },
                    Match {
                        path: "file2.txt".to_string(),
                        result: Some(MatchResult::Found {
                            line_number: Some(3),
                            line: "Hello universe".to_string(),
                        }),
                    },
                ],
            }),
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_fs_search_no_matches() {
        let fixture = ToolOperation::FsSearch {
            input: forge_domain::FSSearch {
                path: Some("/home/user/project".to_string()),
                pattern: "nonexistent".to_string(),
                ..Default::default()
            },
            output: Some(SearchResult {
                matches: vec![Match {
                    path: "file1.txt".to_string(),
                    result: Some(MatchResult::Error("Permission denied".to_string())),
                }],
            }),
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_fs_search_none() {
        let fixture = ToolOperation::FsSearch {
            input: forge_domain::FSSearch {
                path: Some("/home/user/project".to_string()),
                pattern: "search".to_string(),
                ..Default::default()
            },
            output: None,
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_fs_patch_success() {
        let after_content = "Hello universe\nThis is a test\nNew line";
        let fixture = ToolOperation::FsPatch {
            input: forge_domain::FSPatch {
                file_path: "/home/user/project/test.txt".to_string(),
                old_string: "Hello world".to_string(),
                new_string: "Hello universe".to_string(),
                replace_all: false,
            },
            output: PatchOutput {
                errors: vec![],
                before: "Hello world\nThis is a test".to_string(),
                after: after_content.to_string(),
                content_hash: crate::compute_hash(after_content),
            },
        };
        let env = fixture_environment();
        let actual = fixture.to_content(&env).unwrap();
        let actual = strip_ansi_codes(actual.as_str());
        assert_snapshot!(actual)
    }

    #[test]
    fn test_fs_patch_with_warning() {
        let after_content = "line1\nnew line\nline2";
        let fixture = ToolOperation::FsPatch {
            input: forge_domain::FSPatch {
                file_path: "/home/user/project/large_file.txt".to_string(),
                old_string: "line2".to_string(),
                new_string: "new line\nline2".to_string(),
                replace_all: false,
            },
            output: PatchOutput {
                errors: vec![forge_domain::SyntaxError {
                    line: 10,
                    column: 5,
                    message: "Syntax error".to_string(),
                }],
                before: "line1\nline2".to_string(),
                after: after_content.to_string(),
                content_hash: crate::compute_hash(after_content),
            },
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);

        // Should return Some(String) with formatted diff output
        assert!(actual.is_some());
        let output = actual.unwrap();
        assert!(output.contains("line1"));
        assert!(output.contains("new line"));
    }

    #[test]
    fn test_fs_undo() {
        let fixture = ToolOperation::FsUndo {
            input: forge_domain::FSUndo { path: "/home/user/project/test.txt".to_string() },
            output: FsUndoOutput {
                before_undo: Some("ABC".to_string()),
                after_undo: Some("PQR".to_string()),
            },
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_net_fetch_success() {
        let fixture = ToolOperation::NetFetch {
            input: forge_domain::NetFetch {
                url: "https://example.com".to_string(),
                raw: Some(false),
            },
            output: HttpResponse {
                content: "# Example Website\n\nThis is content.".to_string(),
                code: 200,
                context: ResponseContext::Parsed,
                content_type: "text/html".to_string(),
            },
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_net_fetch_error() {
        let fixture = ToolOperation::NetFetch {
            input: forge_domain::NetFetch {
                url: "https://example.com/notfound".to_string(),
                raw: Some(true),
            },
            output: HttpResponse {
                content: "Not Found".to_string(),
                code: 404,
                context: ResponseContext::Raw,
                content_type: "text/plain".to_string(),
            },
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_shell_success() {
        let fixture = ToolOperation::Shell {
            output: ShellOutput {
                output: forge_domain::CommandOutput {
                    command: "ls -la".to_string(),
                    stdout: "file1.txt\nfile2.txt".to_string(),
                    stderr: "".to_string(),
                    exit_code: Some(0),
                },
                shell: "/bin/bash".to_string(),
                description: None,
            },
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_shell_success_with_stderr() {
        let fixture = ToolOperation::Shell {
            output: ShellOutput {
                output: forge_domain::CommandOutput {
                    command: "command_with_warnings".to_string(),
                    stdout: "output line".to_string(),
                    stderr: "warning line".to_string(),
                    exit_code: Some(0),
                },
                shell: "/bin/bash".to_string(),
                description: None,
            },
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_shell_failure() {
        let fixture = ToolOperation::Shell {
            output: ShellOutput {
                output: forge_domain::CommandOutput {
                    command: "failing_command".to_string(),
                    stdout: "".to_string(),
                    stderr: "Error: command not found".to_string(),
                    exit_code: Some(127),
                },
                shell: "/bin/bash".to_string(),
                description: None,
            },
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_follow_up_with_response() {
        let fixture = ToolOperation::FollowUp {
            output: Some("Yes, continue with the operation".to_string()),
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_follow_up_no_response() {
        let fixture = ToolOperation::FollowUp { output: None };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        let expected = None;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_plan_create() {
        let fixture = ToolOperation::PlanCreate {
            input: forge_domain::PlanCreate {
                plan_name: "test-plan".to_string(),
                version: "v1".to_string(),
                content:
                    "# Test Plan\n\n## Task 1\n- Do something\n\n## Task 2\n- Do something else"
                        .to_string(),
            },
            output: crate::PlanCreateOutput {
                path: PathBuf::from("plans/2024-08-11-test-plan-v1.md"),
                before: None,
            },
        };
        let env = fixture_environment();

        let actual = fixture.to_content(&env);
        if let Some(ChatResponseContent::ToolInput(title)) = actual {
            assert_eq!(title.title, "Create plans/2024-08-11-test-plan-v1.md");
            assert_eq!(title.category, forge_domain::Category::Debug);
            assert_eq!(title.sub_title, None);
        } else {
            panic!("Expected Title content");
        }
    }
}
