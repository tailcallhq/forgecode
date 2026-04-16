use crate::confirm::ConfirmBuilder;
use crate::input::InputBuilder;
use crate::multi::MultiSelectBuilder;
use crate::select::SelectBuilder;

/// Centralized fzf-based select functionality with consistent error handling.
///
/// All interactive selection is delegated to the external `fzf` binary.
/// Requires `fzf` to be installed on the system.
pub struct ForgeWidget;

impl ForgeWidget {
    /// Entry point for select operations with fuzzy search.
    pub fn select<T>(message: impl Into<String>, options: Vec<T>) -> SelectBuilder<T> {
        SelectBuilder {
            message: message.into(),
            options,
            starting_cursor: None,
            default: None,
            help_message: None,
            initial_text: None,
            header_lines: 0,
            preview: None,
            preview_window: None,
        }
    }

    /// Convenience method for confirm (yes/no).
    pub fn confirm(message: impl Into<String>) -> ConfirmBuilder {
        ConfirmBuilder { message: message.into(), default: None }
    }

    /// Prompt a question and get text input.
    pub fn input(message: impl Into<String>) -> InputBuilder {
        InputBuilder {
            message: message.into(),
            allow_empty: false,
            default: None,
            default_display: None,
        }
    }

    /// Multi-select prompt.
    pub fn multi_select<T>(message: impl Into<String>, options: Vec<T>) -> MultiSelectBuilder<T> {
        MultiSelectBuilder { message: message.into(), options }
    }
}
