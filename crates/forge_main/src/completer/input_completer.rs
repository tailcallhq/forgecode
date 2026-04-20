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
            // if the line starts with '/' or ':' it's probably a command, so we delegate to
            // the command completer.
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

            let preview_cmd = build_preview_cmd(cat_cmd_for_preview());

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

/// Prefers `bat` for syntax-highlighted output when it is on `$PATH`, falling
/// back to plain `cat`. Mirrors the shell plugin's `_FORGE_CAT_CMD`.
fn cat_cmd_for_preview() -> &'static str {
    if which_bat() {
        "bat --color=always --style=numbers,changes --line-range=:500"
    } else {
        "cat"
    }
}

/// Builds the fzf `--preview` command as `sh -c '…' _ {2}`.
///
/// Wrapping in `sh -c` is required because fzf dispatches previews through
/// `$SHELL`, and shells like fish cannot parse the POSIX `if/then/fi` body.
/// `{2}` is fzf's substitution for the tab-separated path column.
fn build_preview_cmd(cat_cmd: &str) -> String {
    format!(
        r#"sh -c 'if [ -d "$1" ]; then ls -la --color=always "$1" 2>/dev/null || ls -la "$1"; else {cat_cmd} "$1"; fi' _ {{2}}"#
    )
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_build_preview_cmd_with_cat() {
        let fixture = "cat";
        let actual = build_preview_cmd(fixture);
        let expected = r#"sh -c 'if [ -d "$1" ]; then ls -la --color=always "$1" 2>/dev/null || ls -la "$1"; else cat "$1"; fi' _ {2}"#;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_build_preview_cmd_with_bat() {
        let fixture = "bat --color=always --style=numbers,changes --line-range=:500";
        let actual = build_preview_cmd(fixture);
        let expected = r#"sh -c 'if [ -d "$1" ]; then ls -la --color=always "$1" 2>/dev/null || ls -la "$1"; else bat --color=always --style=numbers,changes --line-range=:500 "$1"; fi' _ {2}"#;
        assert_eq!(actual, expected);
    }
}
