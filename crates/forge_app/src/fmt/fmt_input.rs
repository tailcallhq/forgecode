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
                    (true, true) => format!("Overwrite: {display_path}"),
                    (true, false) => {
                        return None;
                    }
                    (false, _) => format!("Create: {display_path}"),
                };
                // Show the content inline so the user has context about what will be
                // written before any permission prompt
                let body = if input.content.is_empty() {
                    "[empty content]".to_string()
                } else {
                    format!("{title}\n\n{}", input.content)
                };
                Some(ChatResponseContent::ToolOutput(body))
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
                let body = format!(
                    "{operation_name}: {display_path}\n\n--- old string ---\n{}\n--- new string ---\n{}",
                    input.old_string, input.new_string
                );
                Some(ChatResponseContent::ToolOutput(body))
            }
            ToolCatalog::MultiPatch(input) => {
                let display_path = display_path_for(&input.file_path);
                let edits: Vec<String> = input
                    .edits
                    .iter()
                    .map(|e| {
                        format!(
                            "- Replace \"{}…\" → \"{}…\"{}",
                            &e.old_string[..e.old_string.len().min(60)],
                            &e.new_string[..e.new_string.len().min(60)],
                            if e.replace_all { " (all occurrences)" } else { "" }
                        )
                    })
                    .collect();
                Some(ChatResponseContent::ToolOutput(format!(
                    "Replace: {display_path} ({} edit(s))\n\n{}",
                    input.edits.len(),
                    edits.join("\n")
                )))
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
