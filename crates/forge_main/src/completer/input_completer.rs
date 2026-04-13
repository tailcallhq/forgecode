use std::path::PathBuf;
use std::sync::Arc;

use forge_select::ForgeWidget;
use forge_walker::Walker;
use reedline::{Completer, Span, Suggestion};

use crate::completer::CommandCompleter;
use crate::completer::search_term::SearchTerm;
use crate::model::ForgeCommandManager;

pub struct InputCompleter {
    cwd: PathBuf,
    command: CommandCompleter,
}

impl InputCompleter {
    pub fn new(cwd: PathBuf, command_manager: Arc<ForgeCommandManager>) -> Self {
        Self { cwd, command: CommandCompleter::new(command_manager) }
    }
}

impl Completer for InputCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        if line.starts_with('/') || line.starts_with(':') {
            // if the line starts with '/' or ':' it's probably a command, so we delegate to the
            // command completer.
            let result = self.command.complete(line, pos);
            if !result.is_empty() {
                return result;
            }
        }

        if let Some(query) = SearchTerm::new(line, pos).process() {
            let walker = Walker::max_all().cwd(self.cwd.clone()).skip_binary(true);
            let files: Vec<String> = walker
                .get_blocking()
                .unwrap_or_default()
                .into_iter()
                .map(|file| file.path)
                .collect();

            // Preview command: show directory listing for dirs, file contents for files.
            // {2} references the path column (items are formatted as "{idx}\t{path}").
            // Use bat for syntax-highlighted file previews when available, falling back
            // to cat. Mirrors the shell plugin's _FORGE_CAT_CMD and completion.zsh preview.
            let cat_cmd = if which_bat() {
                "bat --color=always --style=numbers,changes --line-range=:500"
            } else {
                "cat"
            };
            let preview_cmd = format!(
                "if [ -d {{2}} ]; then ls -la --color=always {{2}} 2>/dev/null || ls -la {{2}}; else {cat_cmd} {{2}}; fi"
            );

            let mut builder = ForgeWidget::select("File", files)
                .with_preview(preview_cmd)
                .with_preview_window("bottom:75%:wrap:border-sharp");
            if !query.term.is_empty() {
                builder = builder.with_initial_text(query.term);
            }

            if let Ok(Some(selected)) = builder.prompt() {
                let value = format!("[{}]", selected);
                return vec![Suggestion {
                    description: None,
                    value,
                    style: None,
                    extra: None,
                    span: Span::new(query.span.start, query.span.end),
                    append_whitespace: true,
                    match_indices: None,
                    display_override: None,
                }];
            }
        }

        vec![]
    }
}

/// Returns `true` if the `bat` binary is available on `PATH`.
fn which_bat() -> bool {
    std::process::Command::new("which")
        .arg("bat")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
