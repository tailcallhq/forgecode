use std::io::IsTerminal;

use anyhow::Result;
use console::strip_ansi_codes;
use nucleo_picker::PickerOptions;
use nucleo_picker::error::PickError;
use nucleo_picker::render::StrRenderer;

/// Builder for select prompts with fuzzy search.
pub struct SelectBuilder<T> {
    pub(crate) message: String,
    pub(crate) options: Vec<T>,
    pub(crate) starting_cursor: Option<usize>,
    pub(crate) default: Option<bool>,
    pub(crate) help_message: Option<&'static str>,
    pub(crate) initial_text: Option<String>,
    pub(crate) header_lines: usize,
    pub(crate) preview: Option<String>,
    pub(crate) preview_window: Option<String>,
}

impl<T: 'static> SelectBuilder<T> {
    /// Set starting cursor position.
    pub fn with_starting_cursor(mut self, cursor: usize) -> Self {
        self.starting_cursor = Some(cursor);
        self
    }

    /// Set a preview command shown in a side panel as the user navigates items.
    ///
    /// This is a no-op with nucleo-picker and is retained for API
    /// compatibility.
    pub fn with_preview(mut self, _command: impl Into<String>) -> Self {
        self.preview = Some(_command.into());
        self
    }

    /// Set the layout of the preview panel.
    ///
    /// This is a no-op with nucleo-picker and is retained for API
    /// compatibility.
    pub fn with_preview_window(mut self, _layout: impl Into<String>) -> Self {
        self.preview_window = Some(_layout.into());
        self
    }

    /// Set default for confirm (only works with bool options).
    pub fn with_default(mut self, default: bool) -> Self {
        self.default = Some(default);
        self
    }

    /// Set help message displayed as a header above the list.
    pub fn with_help_message(mut self, message: &'static str) -> Self {
        self.help_message = Some(message);
        self
    }

    /// Set initial search text for fuzzy search.
    pub fn with_initial_text(mut self, text: impl Into<String>) -> Self {
        self.initial_text = Some(text.into());
        self
    }

    /// Set the number of header lines (non-selectable) at the top of the list.
    ///
    /// Header lines are printed before the picker but are not injected as
    /// selectable items.
    pub fn with_header_lines(mut self, n: usize) -> Self {
        self.header_lines = n;
        self
    }

    /// Execute select prompt with fuzzy search.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(T))` - User selected an option
    /// - `Ok(None)` - No options available or user cancelled (ESC / Ctrl+C)
    ///
    /// # Errors
    ///
    /// Returns an error if the picker fails to start or interact.
    pub fn prompt(self) -> Result<Option<T>>
    where
        T: std::fmt::Display + Clone,
    {
        if !std::io::stderr().is_terminal() {
            return Ok(None);
        }

        if std::any::TypeId::of::<T>() == std::any::TypeId::of::<bool>() {
            return prompt_confirm_as(&self.message, self.default);
        }

        if self.options.is_empty() {
            return Ok(None);
        }

        let display_options: Vec<String> = self
            .options
            .iter()
            .map(|item| strip_ansi_codes(&item.to_string()).trim().to_string())
            .collect();

        let header_count = self.header_lines.min(display_options.len());
        let data_items = display_options
            .into_iter()
            .skip(header_count)
            .collect::<Vec<_>>();

        if data_items.is_empty() {
            return Ok(None);
        }

        let mut picker_opts = PickerOptions::default()
            .reversed(true)
            .case_matching(nucleo_picker::CaseMatching::Smart);

        if let Some(query) = &self.initial_text {
            picker_opts = picker_opts.query(query.clone());
        }

        let mut picker: nucleo_picker::Picker<String, _> = picker_opts.picker(StrRenderer);

        if let Some(cursor) = self.starting_cursor {
            let effective_cursor = cursor;
            if effective_cursor > 0 && effective_cursor < data_items.len() {
                let mut reordered = data_items;
                reordered.swap(0, effective_cursor);
                picker.extend_exact(reordered);
            } else {
                picker.extend_exact(data_items);
            }
        } else {
            picker.extend_exact(data_items);
        }

        if let Some(help) = self.help_message {
            eprintln!("{}", help);
        }
        for header in self.options.iter().take(header_count) {
            eprintln!("{}", header);
        }

        match picker.pick() {
            Ok(Some(selected)) => {
                let selected_str: &str = selected.as_ref();
                Ok(self
                    .options
                    .iter()
                    .skip(header_count)
                    .find(|opt| strip_ansi_codes(&opt.to_string()).trim() == selected_str)
                    .cloned())
            }
            Ok(None) => Ok(None),
            Err(PickError::NotInteractive) => Ok(None),
            Err(PickError::UserInterrupted) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("Picker error: {e}")),
        }
    }
}

/// Runs a yes/no confirmation prompt via nucleo-picker.
///
/// Returns `Ok(Some(true))` for Yes, `Ok(Some(false))` for No, and `Ok(None)`
/// if cancelled.
fn prompt_confirm(message: &str, default: Option<bool>) -> Result<Option<bool>> {
    let items = if default == Some(false) {
        vec!["No".to_string(), "Yes".to_string()]
    } else {
        vec!["Yes".to_string(), "No".to_string()]
    };

    let mut picker: nucleo_picker::Picker<String, _> =
        PickerOptions::default().reversed(true).picker(StrRenderer);

    picker.extend_exact(items);

    eprintln!("{}", message);

    match picker.pick() {
        Ok(Some(selected)) => {
            let result = match selected.as_str() {
                "Yes" => Some(true),
                "No" => Some(false),
                _ => None,
            };
            Ok(result)
        }
        Ok(None) => Ok(None),
        Err(PickError::NotInteractive) => Ok(None),
        Err(PickError::UserInterrupted) => Ok(None),
        Err(e) => Err(anyhow::anyhow!("Picker error: {e}")),
    }
}

/// Wrapper around [`prompt_confirm`] that safely converts the `bool` result
/// into the generic type `T`.
///
/// This must only be called when `T` is known to be `bool` (verified via
/// `TypeId` at the call site). The conversion uses `Any` downcasting instead
/// of `transmute_copy` to remain fully safe.
fn prompt_confirm_as<T: 'static + Clone>(
    message: &str,
    default: Option<bool>,
) -> Result<Option<T>> {
    let result = prompt_confirm(message, default)?;
    Ok(result.and_then(|value| {
        let any_value: Box<dyn std::any::Any> = Box::new(value);
        any_value.downcast::<T>().ok().map(|boxed| *boxed)
    }))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::ForgeWidget;

    #[test]
    fn test_select_builder_creates() {
        let builder = ForgeWidget::select("Test", vec!["a", "b", "c"]);
        assert_eq!(builder.message, "Test");
        assert_eq!(builder.options, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_confirm_builder_creates() {
        let builder = ForgeWidget::confirm("Confirm?");
        assert_eq!(builder.message, "Confirm?");
    }

    #[test]
    fn test_select_builder_with_initial_text() {
        let builder =
            ForgeWidget::select("Test", vec!["apple", "banana", "cherry"]).with_initial_text("app");
        assert_eq!(builder.initial_text, Some("app".to_string()));
    }

    #[test]
    fn test_select_owned_builder_with_initial_text() {
        let builder =
            ForgeWidget::select("Test", vec!["apple", "banana", "cherry"]).with_initial_text("ban");
        assert_eq!(builder.initial_text, Some("ban".to_string()));
    }

    #[test]
    fn test_ansi_stripping() {
        let options = ["\x1b[1mBold\x1b[0m", "\x1b[31mRed\x1b[0m"];
        let display: Vec<String> = options
            .iter()
            .map(|value| strip_ansi_codes(value).to_string())
            .collect();

        assert_eq!(display, vec!["Bold", "Red"]);
    }

    #[test]
    fn test_display_options_are_trimmed() {
        let fixture = [
            "  openai               [empty]",
            "✓ anthropic            [api.anthropic.com]",
        ];
        let actual: Vec<String> = fixture
            .iter()
            .map(|value| strip_ansi_codes(value).trim().to_string())
            .collect();
        let expected = vec![
            "openai               [empty]".to_string(),
            "✓ anthropic            [api.anthropic.com]".to_string(),
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_with_starting_cursor() {
        let builder = ForgeWidget::select("Test", vec!["a", "b", "c"]).with_starting_cursor(2);
        assert_eq!(builder.starting_cursor, Some(2));
    }
}
