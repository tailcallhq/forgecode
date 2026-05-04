use std::io::IsTerminal;

use anyhow::Result;
use console::strip_ansi_codes;

use crate::preview::{PreviewLayout, PreviewPlacement, SelectMode, SelectRow, SelectUiOptions};

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
    pub fn with_preview(mut self, command: impl Into<String>) -> Self {
        self.preview = Some(command.into());
        self
    }

    /// Set the layout of the preview panel.
    pub fn with_preview_window(mut self, layout: impl Into<String>) -> Self {
        self.preview_window = Some(layout.into());
        self
    }

    /// Set default for confirm prompts using bool options.
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

    /// Set the number of header lines treated as non-selectable options.
    pub fn with_header_lines(mut self, n: usize) -> Self {
        self.header_lines = n;
        self
    }

    /// Execute select prompt with fuzzy search.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(T))` when the user selects an option.
    /// - `Ok(None)` when no options are available or the user cancels.
    ///
    /// # Errors
    ///
    /// Returns an error if the picker cannot set up terminal interaction,
    /// render, process events, or run a preview command.
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

        let rows = self
            .options
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let display = strip_ansi_codes(&item.to_string()).trim().to_string();
                if index < self.header_lines {
                    SelectRow::header(display)
                } else {
                    SelectRow::new(index.to_string(), display.clone()).search(display)
                }
            })
            .collect::<Vec<_>>();

        let header_count = self.header_lines.min(rows.len());
        if rows.len() == header_count {
            return Ok(None);
        }

        let mut selector = SelectUiOptions::new(format!("{} ❯ ", self.message), rows)
            .header_lines(header_count)
            .mode(SelectMode::Single)
            .preview_layout(parse_preview_layout(self.preview_window.as_deref()));

        if let Some(query) = self.initial_text {
            selector = selector.query(Some(query));
        }

        if let Some(preview) = self.preview {
            selector = selector.preview(Some(preview));
        }

        if let Some(cursor) = self.starting_cursor {
            selector = selector.initial_raw(Some(cursor.to_string()));
        }

        if let Some(help) = self.help_message {
            selector.rows.insert(0, SelectRow::header(help));
            selector.header_lines = selector.header_lines.saturating_add(1);
        }

        let selected = selector.prompt()?;
        Ok(selected.and_then(|row| {
            row.raw
                .parse::<usize>()
                .ok()
                .and_then(|index| self.options.get(index).cloned())
        }))
    }
}

fn parse_preview_layout(layout: Option<&str>) -> PreviewLayout {
    let Some(layout) = layout else {
        return PreviewLayout::default();
    };

    let placement = if layout.contains("down") || layout.contains("bottom") {
        PreviewPlacement::Bottom
    } else {
        PreviewPlacement::Right
    };

    let percent = layout
        .split(|ch: char| !ch.is_ascii_digit())
        .find_map(|part| part.parse::<u16>().ok())
        .unwrap_or_else(|| PreviewLayout::default().percent)
        .clamp(1, 99);

    PreviewLayout { placement, percent }
}

/// Runs a yes/no confirmation prompt.
///
/// Returns `Ok(Some(true))` for Yes, `Ok(Some(false))` for No, and `Ok(None)`
/// if cancelled.
fn prompt_confirm(message: &str, default: Option<bool>) -> Result<Option<bool>> {
    let rows = if default == Some(false) {
        vec![SelectRow::new("no", "No"), SelectRow::new("yes", "Yes")]
    } else {
        vec![SelectRow::new("yes", "Yes"), SelectRow::new("no", "No")]
    };

    let selected = SelectUiOptions::new(format!("{} ❯ ", message), rows).prompt()?;
    Ok(selected.and_then(|row| match row.raw.as_str() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }))
}

/// Wrapper around [`prompt_confirm`] that safely converts the `bool` result
/// into the generic type `T`.
///
/// This must only be called when `T` is known to be `bool`.
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
        let fixture = ["\x1b[1mBold\x1b[0m", "\x1b[31mRed\x1b[0m"];
        let actual: Vec<String> = fixture
            .iter()
            .map(|value| strip_ansi_codes(value).to_string())
            .collect();
        let expected = vec!["Bold", "Red"];
        assert_eq!(actual, expected);
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

    #[test]
    fn test_parse_preview_layout_defaults_to_right() {
        let fixture = None;
        let actual = parse_preview_layout(fixture);
        let expected = PreviewLayout { placement: PreviewPlacement::Right, percent: 50 };
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_parse_preview_layout_supports_bottom_percent() {
        let fixture = Some("down,60%");
        let actual = parse_preview_layout(fixture);
        let expected = PreviewLayout { placement: PreviewPlacement::Bottom, percent: 60 };
        assert_eq!(actual, expected);
    }
}
