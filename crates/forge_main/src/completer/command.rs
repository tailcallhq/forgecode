use std::sync::Arc;

use forge_select::ForgeWidget;
use reedline::{Completer, Span, Suggestion};

use crate::model::{ForgeCommand, ForgeCommandManager};

/// A display wrapper for `ForgeCommand` that renders the name and description
/// side-by-side for fzf.
#[derive(Clone)]
struct CommandRow(ForgeCommand);

impl std::fmt::Display for CommandRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:<30} {}", self.0.name, self.0.description)
    }
}

#[derive(Clone)]
pub struct CommandCompleter(Arc<ForgeCommandManager>);

impl CommandCompleter {
    pub fn new(command_manager: Arc<ForgeCommandManager>) -> Self {
        Self(command_manager)
    }
}

impl Completer for CommandCompleter {
    fn complete(&mut self, line: &str, _: usize) -> Vec<reedline::Suggestion> {
        // Determine which sentinel the user typed (`:` or `/`), defaulting to `/`.
        let sentinel = if line.starts_with(':') { ':' } else { '/' };

        // Build the list of display names using the same sentinel the user typed.
        let commands: Vec<CommandRow> = self
            .0
            .list()
            .into_iter()
            .filter_map(|cmd| {
                let display_name = if cmd.name.starts_with('!') {
                    cmd.name.clone()
                } else {
                    format!("{}{}", sentinel, cmd.name)
                };

                // Only include commands that match what the user has typed so far.
                if display_name.starts_with(line) {
                    Some(CommandRow(ForgeCommand {
                        name: display_name,
                        description: cmd.description,
                        value: cmd.value,
                    }))
                } else {
                    None
                }
            })
            .collect();

        if commands.is_empty() {
            return vec![];
        }

        // Extract the initial query text (everything after the leading sentinel or `!`).
        let initial_query = line
            .strip_prefix('/')
            .or_else(|| line.strip_prefix(':'))
            .or_else(|| line.strip_prefix('!'))
            .unwrap_or(line);

        let mut builder = ForgeWidget::select("Command", commands);
        if !initial_query.is_empty() {
            builder = builder.with_initial_text(initial_query);
        }

        match builder.prompt() {
            Ok(Some(row)) => {
                vec![Suggestion {
                    value: row.0.name,
                    description: None,
                    style: None,
                    extra: None,
                    span: Span::new(0, line.len()),
                    append_whitespace: true,
                    match_indices: None,
                    display_override: None,
                }]
            }
            _ => vec![],
        }
    }
}
