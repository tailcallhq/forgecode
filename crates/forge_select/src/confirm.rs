use anyhow::Result;
use colored::Colorize;

use crate::input::InputBuilder;

/// Builder for confirm (yes/no) prompts.
pub struct ConfirmBuilder {
    pub(crate) message: String,
    pub(crate) default: Option<bool>,
}

impl ConfirmBuilder {
    /// Set the default value for the confirm prompt.
    ///
    /// If the user presses Enter without typing anything, this default will be
    /// used.
    pub fn with_default(mut self, default: bool) -> Self {
        self.default = Some(default);
        self
    }

    /// Execute the confirm prompt.
    ///
    /// Prompts the user with the message and expects Y/y/yes or N/no.
    /// If the user enters an empty response and a default is set, the default
    /// is used. If the input cannot be converted to yes/no, the prompt is
    /// repeated in a loop until a valid response or cancellation is received.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(true))` - User confirmed (Y, y, yes, YES, etc.)
    /// - `Ok(Some(false))` - User denied (N, n, no, NO, etc.)
    /// - `Ok(None)` - User cancelled (EOF / Ctrl+D / Ctrl+C)
    /// - `Err(...)` - If the prompt fails
    pub fn prompt(self) -> Result<Option<bool>> {
        let hint = match self.default {
            Some(true) => "Y/n".to_string(),
            Some(false) => "y/N".to_string(),
            None => "y/n".to_string(),
        };

        let message_with_hint = if cfg!(windows) {
            format!("{} {}", self.message, hint)
        } else {
            format!("{} {}", self.message, hint.yellow())
        };

        loop {
            let input_builder = InputBuilder {
                message: message_with_hint.clone(),
                allow_empty: true,
                default: None,
                default_display: None,
                header: None,
            };

            let result = input_builder.prompt()?;

            // User cancelled (Ctrl+C or EOF)
            if result.is_none() {
                return Ok(None);
            }

            let input = result.unwrap().trim().to_lowercase();

            // Empty input - use default
            if input.is_empty() {
                return Ok(Some(self.default.unwrap_or(false)));
            }

            // Parse Y/N response
            if input == "y" || input == "yes" {
                return Ok(Some(true));
            }
            if input == "n" || input == "no" {
                return Ok(Some(false));
            }
        }
    }
}
