use std::io::{self, IsTerminal};

use anyhow::Result;
use colored::Colorize;
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use rustyline::DefaultEditor;
use tracing::debug;

/// Strips bracketed-paste escape sequences from a string.
///
/// When bracketed paste mode is active in the terminal, pasted text is wrapped
/// in `\x1b[200~` (start) and `\x1b[201~` (end) markers. This function removes
/// those markers from the captured shell output so the raw input value is
/// clean.
fn strip_bracketed_paste(s: &str) -> String {
    s.replace("\x1b[200~", "").replace("\x1b[201~", "")
}

/// Builder for input prompts.
pub struct InputBuilder {
    pub(crate) message: String,
    pub(crate) allow_empty: bool,
    pub(crate) default: Option<String>,
    pub(crate) default_display: Option<String>,
}

impl InputBuilder {
    /// Allow empty input.
    pub fn allow_empty(mut self, allow: bool) -> Self {
        self.allow_empty = allow;
        self
    }

    /// Set default value.
    pub fn with_default<T>(mut self, default: T) -> Self
    where
        T: std::fmt::Display + AsRef<str>,
    {
        self.default = Some(default.as_ref().to_string());
        self.default_display = Some(default.to_string());
        self
    }

    /// Execute input prompt using rustyline.
    ///
    /// Uses `rustyline::DefaultEditor` to provide full line editing (backspace,
    /// arrow keys, Ctrl+A/E, etc.). Requires stdin to be a real tty — the
    /// caller is responsible for ensuring this (e.g. via `</dev/tty` when
    /// launched from a ZLE widget). When `allow_empty` is false and no default
    /// is set, re-prompts until non-empty input is provided.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(String))` - User provided input
    /// - `Ok(None)` - User cancelled (EOF / Ctrl+D / Ctrl+C)
    ///
    /// # Errors
    ///
    /// Returns an error if rustyline fails to initialise or read input.
    pub fn prompt(self) -> Result<Option<String>> {
        // Bail immediately when stdin is not a terminal to prevent the process
        // from blocking indefinitely on a detached or non-interactive session.
        if !std::io::stdin().is_terminal() {
            return Ok(None);
        }

        // Enter the alternate screen so that the prompt is always visible and
        // cannot be scrolled out of the viewport. This fixes an issue in
        // terminals like VS Code (xterm.js) where rustyline's per-keystroke
        // redraw causes the viewport to jump back to the cursor position,
        // scrolling the prompt out of view.
        let _guard = AlternateScreenGuard::enter();

        let mut rl = DefaultEditor::new()?;

        // On Windows, rustyline miscounts ANSI escape bytes as visible characters,
        // causing incorrect cursor placement and extra space before the editor.
        let prompt_str = if cfg!(windows) {
            format!("? {}: ", self.message)
        } else {
            format!("{} {}: ", "?".yellow().bold(), self.message.bold())
        };

        let initial = self.default.as_deref().unwrap_or("");

        loop {
            let readline = rl.readline_with_initial(&prompt_str, (initial, ""));
            debug!(output = ?readline, "Readline input");
            let line = match readline {
                Ok(s) => s,
                Err(rustyline::error::ReadlineError::Eof)
                | Err(rustyline::error::ReadlineError::Interrupted) => return Ok(None),
                Err(e) => return Err(e.into()),
            };

            let value = strip_bracketed_paste(&line);
            let trimmed = value.trim();

            if trimmed.is_empty() {
                if let Some(ref default_val) = self.default {
                    return Ok(Some(default_val.clone()));
                }
                if self.allow_empty {
                    return Ok(Some(String::new()));
                }
                continue;
            }

            return Ok(Some(trimmed.to_string()));
        }
    }
}

/// Guard that enters the terminal alternate screen on creation and exits it on
/// drop. Failures are silently ignored — the alternate screen is a cosmetic
/// best-effort fix for terminal viewport issues.
struct AlternateScreenGuard;

impl AlternateScreenGuard {
    fn enter() -> Option<Self> {
        execute!(io::stdout(), EnterAlternateScreen).ok()?;
        Some(Self)
    }
}

impl Drop for AlternateScreenGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::ForgeWidget;

    #[test]
    fn test_input_builder_creates() {
        let builder = ForgeWidget::input("Enter name");
        assert_eq!(builder.message, "Enter name");
        assert_eq!(builder.allow_empty, false);
    }

    #[test]
    fn test_input_builder_with_default() {
        let builder = ForgeWidget::input("Enter key").with_default("mykey");
        assert_eq!(builder.default, Some("mykey".to_string()));
    }

    #[test]
    fn test_input_builder_allow_empty() {
        let builder = ForgeWidget::input("Enter").allow_empty(true);
        assert_eq!(builder.allow_empty, true);
    }

    #[test]
    fn test_strip_bracketed_paste() {
        let fixture = "\x1b[200~myapikey\x1b[201~";
        let actual = strip_bracketed_paste(fixture);
        let expected = "myapikey";
        assert_eq!(actual, expected);

        let fixture = "myapikey";
        let actual = strip_bracketed_paste(fixture);
        let expected = "myapikey";
        assert_eq!(actual, expected);

        let fixture = "\x1b[200~myapikey";
        let actual = strip_bracketed_paste(fixture);
        let expected = "myapikey";
        assert_eq!(actual, expected);
    }
}
