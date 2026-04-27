use std::path::{Path, PathBuf};
use std::sync::Arc;

use forge_walker::Walker;
use reedline::{Completer, Span, Suggestion};

use crate::completer::CommandCompleter;
use crate::completer::search_term::SearchTerm;
use crate::model::ForgeCommandManager;
use crate::select_cmd::{
    PreviewLayout, PreviewPlacement, SelectMode, SelectRow, SelectUiOptions, redirect_stdin_to_tty,
    run_select_ui,
};

pub fn select_workspace_file(cwd: &Path, query: Option<String>) -> anyhow::Result<Option<String>> {
    #[cfg(unix)]
    {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            redirect_stdin_to_tty()?;
        }
    }

    let files: Vec<String> = Walker::max_all()
        .cwd(cwd.to_path_buf())
        .skip_binary(true)
        .get_blocking()
        .unwrap_or_default()
        .into_iter()
        .map(|file| file.path)
        .collect();

    if files.is_empty() {
        return Ok(None);
    }

    let has_bat = std::process::Command::new("bat")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok();
    let cat_cmd = if has_bat {
        "bat --color=always --style=numbers,changes --line-range=:500"
    } else {
        "cat"
    };

    let preview_cmd = format!(
        "if [ -d {{}} ]; then ls -la --color=always {{}} 2>/dev/null || ls -la {{}}; else {cat_cmd} {{}}; fi"
    );
    let rows: Vec<SelectRow> = files
        .into_iter()
        .map(|path| SelectRow { raw: path.clone(), display: path.clone(), fields: vec![path] })
        .collect();

    Ok(run_select_ui(SelectUiOptions {
        prompt: Some("File ❯ ".to_string()),
        query,
        rows,
        header_lines: 0,
        mode: SelectMode::Single,
        preview: Some(preview_cmd),
        preview_layout: PreviewLayout { placement: PreviewPlacement::Bottom, percent: 75 },
        initial_raw: None,
    })?)
}

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
            let initial_text = if !query.term.is_empty() {
                Some(query.term.to_string())
            } else {
                None
            };

            if let Ok(Some(selected)) = select_workspace_file(&self.cwd, initial_text) {
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
