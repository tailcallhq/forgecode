use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use console::{measure_text_width, strip_ansi_codes};
use forge_api::Environment;
use nu_ansi_term::Style;
use rustyline::completion::{Completer, Pair};
use rustyline::config::{ColorMode, CompletionType, Config};
use rustyline::error::ReadlineError as RustyReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::{Hinter, HistoryHinter};
use rustyline::history::DefaultHistory;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{
    Cmd, Context as RustylineContext, Editor, EventHandler, Helper, KeyCode, KeyEvent, Modifiers,
    Movement, Prompt as RustylinePrompt,
};

use super::completer::InputCompleter;
use super::zsh::paste::wrap_pasted_text;
use crate::highlighter::ForgeHighlighter;
use crate::model::ForgeCommandManager;
use crate::prompt::ForgePrompt;

const HISTORY_CAPACITY: usize = 1024 * 1024;

/// Interactive terminal editor used by the Forge prompt.
pub struct ForgeEditor {
    editor: Editor<ForgeHelper, DefaultHistory>,
    history_file: PathBuf,
    pending_buffer: Option<String>,
}

/// Result of reading one prompt interaction from the terminal.
pub enum ReadResult {
    Success(String),
    Empty,
    Continue,
    Exit,
}

impl ForgeEditor {
    /// Creates a new interactive editor with history, completion, and
    /// highlighting.
    pub fn new(
        env: Environment,
        custom_history_path: Option<PathBuf>,
        manager: Arc<ForgeCommandManager>,
    ) -> Self {
        let history_file = env.history_path(custom_history_path.as_ref());
        let helper = ForgeHelper::new(env.cwd, manager);
        let config = Config::builder()
            .max_history_size(HISTORY_CAPACITY)
            .expect("rustyline history capacity should be valid")
            .completion_type(CompletionType::List)
            .completion_show_all_if_ambiguous(true)
            .color_mode(ColorMode::Forced)
            .enable_signals(true)
            .build();
        let mut editor = Editor::<ForgeHelper, DefaultHistory>::with_config(config)
            .expect("rustyline editor should initialize for an interactive terminal");
        editor.bind_sequence(
            KeyEvent(KeyCode::Enter, Modifiers::ALT),
            EventHandler::Simple(Cmd::Newline),
        );
        editor.bind_sequence(
            KeyEvent(KeyCode::Char('k'), Modifiers::CTRL),
            EventHandler::Simple(Cmd::Kill(Movement::EndOfLine)),
        );
        editor.bind_sequence(
            KeyEvent(KeyCode::Char('K'), Modifiers::CTRL),
            EventHandler::Simple(Cmd::Kill(Movement::EndOfLine)),
        );
        editor.set_helper(Some(helper));
        let _ = editor.load_history(&history_file);
        Self { editor, history_file, pending_buffer: None }
    }

    /// Reads one logical input from the terminal.
    pub fn prompt(&mut self, prompt: &mut ForgePrompt) -> anyhow::Result<ReadResult> {
        let prompt_text = render_prompt(prompt);
        let initial = self.pending_buffer.take().unwrap_or_default();
        let readline = if initial.is_empty() {
            self.editor.readline(&prompt_text)
        } else {
            self.editor
                .readline_with_initial(&prompt_text, (&initial, ""))
        };
        prompt.refresh();

        match readline {
            Ok(buffer) => {
                let buffer = normalize_input(buffer);
                let trimmed = buffer.trim();
                if trimmed.is_empty() {
                    return Ok(ReadResult::Empty);
                }
                let _ = self.editor.add_history_entry(trimmed);
                let _ = self.editor.save_history(&self.history_file);
                Ok(ReadResult::Success(trimmed.to_string()))
            }
            Err(RustyReadlineError::Interrupted) => Ok(ReadResult::Continue),
            Err(RustyReadlineError::Eof) => Ok(ReadResult::Exit),
            Err(error) => Err(anyhow::anyhow!(ReadLineError(error))),
        }
    }

    /// Sets the buffer content to be pre-filled on the next prompt.
    pub fn set_buffer(&mut self, content: String) {
        self.pending_buffer = Some(content);
    }
}

#[derive(Debug, thiserror::Error)]
#[error("failed to read line from terminal: {0}")]
pub struct ReadLineError(RustyReadlineError);

fn render_prompt(prompt: &ForgePrompt) -> ResponsivePrompt {
    let left = prompt.render_prompt_left();
    let indicator = prompt.render_prompt_indicator();
    let right = prompt.render_prompt_right();
    let right = right.trim_start();

    if right.trim().is_empty() {
        let prompt = format!("{left}{indicator}");
        return ResponsivePrompt { raw: prompt.clone(), styled: prompt };
    }

    if let Some((first_line, remaining)) = left.split_once('\n') {
        let right = render_right_prompt(right);
        return ResponsivePrompt {
            raw: format!("{first_line}\n{remaining}{indicator}"),
            styled: format!("{first_line}{right}\n{remaining}{indicator}"),
        };
    }

    let right = render_right_prompt(right);
    ResponsivePrompt {
        raw: format!("{left}{indicator}"),
        styled: format!("{left}{right}{indicator}"),
    }
}

fn render_right_prompt(right: &str) -> String {
    let width = measure_text_width(strip_ansi_codes(right).as_ref());
    format!("\x1b[s\x1b[999C\x1b[{width}D{right}\x1b[K\x1b[u")
}

struct ResponsivePrompt {
    raw: String,
    styled: String,
}

impl RustylinePrompt for ResponsivePrompt {
    fn raw(&self) -> &str {
        &self.raw
    }

    fn styled(&self) -> &str {
        &self.styled
    }
}

fn normalize_input(input: String) -> String {
    let stripped = input.replace("\x1b[200~", "").replace("\x1b[201~", "");
    wrap_pasted_text(&stripped)
}

struct ForgeHelper {
    completer: Mutex<InputCompleter>,
    highlighter: ForgeHighlighter,
    hinter: HistoryHinter,
}

impl ForgeHelper {
    fn new(cwd: PathBuf, command_manager: Arc<ForgeCommandManager>) -> Self {
        Self {
            completer: Mutex::new(InputCompleter::new(cwd, command_manager)),
            highlighter: ForgeHighlighter,
            hinter: HistoryHinter {},
        }
    }
}

impl Helper for ForgeHelper {}

impl Completer for ForgeHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &RustylineContext<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let mut completer = self
            .completer
            .lock()
            .expect("input completer mutex poisoned");
        let suggestions = completer.complete(line, pos);
        let start = suggestions
            .iter()
            .map(|suggestion| suggestion.span.start)
            .min()
            .unwrap_or(pos);
        let pairs = suggestions
            .into_iter()
            .map(|suggestion| {
                let replacement = if suggestion.append_whitespace {
                    format!("{} ", suggestion.value)
                } else {
                    suggestion.value
                };
                Pair { display: replacement.clone(), replacement }
            })
            .collect();
        Ok((start, pairs))
    }
}

impl Hinter for ForgeHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &RustylineContext<'_>) -> Option<Self::Hint> {
        self.hinter.hint(line, pos, ctx)
    }
}

impl Highlighter for ForgeHelper {
    fn highlight<'l>(&self, line: &'l str, pos: usize) -> Cow<'l, str> {
        let styled = self.highlighter.highlight(line, pos);
        if styled.buffer.is_empty() {
            return Cow::Borrowed(line);
        }

        let mut rendered = String::with_capacity(line.len());
        for (style, text) in styled.buffer {
            if style == Style::new() {
                rendered.push_str(&text);
            } else {
                rendered.push_str(&style.paint(text).to_string());
            }
        }
        Cow::Owned(rendered)
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Owned(Style::new().dimmed().paint(hint).to_string())
    }
}

impl Validator for ForgeHelper {
    fn validate(&self, _ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        Ok(ValidationResult::Valid(None))
    }
}
