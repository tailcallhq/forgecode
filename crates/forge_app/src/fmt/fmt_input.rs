use std::path::{Path, PathBuf};

use forge_domain::{ChatResponseContent, Environment, TitleFormat, ToolCatalog};

use crate::fmt::content::FormatContent;
use crate::utils::format_display_path;

impl FormatContent for ToolCatalog {
    fn to_content(&self, env: &Environment) -> Option<ChatResponseContent> {
        let display_path_for = |path: &str| format_display_path(Path::new(path), env.cwd.as_path());

        match self {
            ToolCatalog::Read(input) => {
                let display_path = display_path_for(&input.file_path);
                let is_explicit_range = input.range.is_some();
                let mut subtitle = display_path;
                if is_explicit_range && let Some(range) = &input.range {
                    match (range.start_line, range.end_line) {
                        (Some(start), Some(end)) => {
                            subtitle.push_str(&format!(":{start}-{end}"));
                        }
                        (Some(start), None) => {
                            subtitle.push_str(&format!(":{start}"));
                        }
                        (None, Some(end)) => {
                            subtitle.push_str(&format!(":1-{end}"));
                        }
                        (None, None) => {}
                    }
                };
                Some(TitleFormat::debug("Read").sub_title(subtitle).into())
            }
            ToolCatalog::Write(input) => {
                let path = PathBuf::from(&input.file_path);
                let display_path = display_path_for(&input.file_path);
                let title = match (path.exists(), input.overwrite) {
                    (true, true) => "Overwrite",
                    (true, false) => {
                        // Case: file exists but overwrite is false then we throw error from tool,
                        // so it's good idea to not print anything on CLI.
                        return None;
                    }
                    (false, _) => "Create",
                };
                Some(TitleFormat::debug(title).sub_title(display_path).into())
            }
            ToolCatalog::FsSearch(input) => {
                let formatted_dir = input.path.as_deref().unwrap_or(".");
                let formatted_dir = display_path_for(formatted_dir);

                let title = match (&input.glob, &input.file_type) {
                    (Some(glob), _) => {
                        format!(
                            "Search for '{}' in '{}' files at {}",
                            input.pattern, glob, formatted_dir
                        )
                    }
                    (None, Some(file_type)) => {
                        format!(
                            "Search for '{}' in {} files at {}",
                            input.pattern, file_type, formatted_dir
                        )
                    }
                    (None, None) => {
                        format!("Search for '{}' at {}", input.pattern, formatted_dir)
                    }
                };
                Some(TitleFormat::debug(title).into())
            }
            ToolCatalog::SemSearch(input) => {
                let pairs: Vec<_> = input
                    .queries
                    .iter()
                    .map(|item| item.query.as_str())
                    .collect();
                Some(
                    TitleFormat::debug("Codebase Search")
                        .sub_title(format!("[{}]", pairs.join(" · ")))
                        .into(),
                )
            }
            ToolCatalog::Remove(input) => {
                let display_path = display_path_for(&input.path);
                Some(TitleFormat::debug("Remove").sub_title(display_path).into())
            }
            ToolCatalog::Patch(input) => {
                let display_path = display_path_for(&input.file_path);
                let operation_name = if input.replace_all {
                    "Replace All"
                } else {
                    "Replace"
                };
                Some(
                    TitleFormat::debug(operation_name)
                        .sub_title(display_path)
                        .into(),
                )
            }
            ToolCatalog::MultiPatch(input) => {
                let display_path = display_path_for(&input.file_path);
                Some(
                    TitleFormat::debug("Replace")
                        .sub_title(format!("{} ({} edits)", display_path, input.edits.len()))
                        .into(),
                )
            }
            ToolCatalog::Undo(input) => {
                let display_path = display_path_for(&input.path);
                Some(TitleFormat::debug("Undo").sub_title(display_path).into())
            }
            ToolCatalog::Shell(input) => Some(
                TitleFormat::debug(format!("Execute [{}]", env.shell))
                    .sub_title(&input.command)
                    .into(),
            ),
            ToolCatalog::Fetch(input) => {
                Some(TitleFormat::debug("GET").sub_title(&input.url).into())
            }
            ToolCatalog::Followup(input) => Some(
                TitleFormat::debug("Follow-up")
                    .sub_title(&input.question)
                    .into(),
            ),
            ToolCatalog::Plan(_) => None,
            ToolCatalog::Skill(input) => Some(
                TitleFormat::debug("Skill")
                    .sub_title(input.name.to_lowercase())
                    .into(),
            ),
            ToolCatalog::TodoWrite(input) => Some(
                TitleFormat::debug("Update Todos")
                    .sub_title(format!("{} item(s)", input.todos.len()))
                    .into(),
            ),
            ToolCatalog::TodoRead(_) => Some(TitleFormat::debug("Read Todos").into()),
            ToolCatalog::Task(input) => {
                Some(TitleFormat::debug("Task").sub_title(&input.agent_id).into())
            }
        }
    }
}
