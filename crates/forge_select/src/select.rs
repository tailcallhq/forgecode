use std::io::IsTerminal;

use anyhow::Result;
use console::strip_ansi_codes;
use fzf_wrapped::{Fzf, Layout, run_with_output};

/// Builder for select prompts with fuzzy search.
pub struct SelectBuilder<T> {
    pub(crate) message: String,
    pub(crate) options: Vec<T>,
    pub(crate) starting_cursor: Option<usize>,
    pub(crate) default: Option<bool>,
    pub(crate) help_message: Option<&'static str>,
    pub(crate) initial_text: Option<String>,
    pub(crate) header_lines: usize,
}

/// Builds an `Fzf` instance with standard layout and an optional header.
///
/// `--height=80%` is always added so fzf runs inline (below the current cursor)
/// rather than switching to the alternate screen buffer. Without this flag fzf
/// uses full-screen mode which enters the alternate screen (`\033[?1049h`),
/// making it appear as though the terminal is cleared. 80% matches the shell
/// plugin's `_forge_fzf` wrapper for a consistent UI.
///
/// Items are always passed as `"{idx}\t{display}"` and fzf is configured with
/// `--delimiter=\t --with-nth=2..` so only the display portion is shown. The
/// index prefix survives in fzf's output and is parsed back to look up the
/// original item by position — this avoids the `position()` ambiguity when
/// multiple items have identical display strings after ANSI stripping.
///
/// When `starting_cursor` is provided, `--bind="load:pos(N)"` is added so fzf
/// pre-positions the cursor on the Nth item (1-based in fzf's `pos()` action).
/// The `load` event is used instead of `start` because items are written to
/// fzf's stdin after the process starts.
///
/// The flags `--exact`, `--cycle`, `--select-1`, `--no-scrollbar`, and
/// `--color=dark,header:bold` mirror the shell plugin's `_forge_fzf` wrapper
/// for a consistent user experience across both entry points.
///
/// The `message` is used as the fzf `--prompt` so the prompt line reads
/// `"Select a model: "` instead of the default `"> "`, placing the question
/// inline with the search cursor (e.g. `Select a model: ❯`). If a
/// `help_message` is provided it is shown as a `--header` above the list.
fn build_fzf(
    message: &str,
    help_message: Option<&str>,
    initial_text: Option<&str>,
    starting_cursor: Option<usize>,
    header_lines: usize,
) -> Fzf {
    let mut builder = Fzf::builder();
    builder.layout(Layout::Reverse);
    builder.no_scrollbar(true);
    builder.prompt(format!("{} ❯ ", message));

    if let Some(help) = help_message {
        builder.header(help);
    }

    let mut args = vec![
        "--height=80%".to_string(),
        "--exact".to_string(),
        "--cycle".to_string(),
        "--select-1".to_string(),
        "--color=dark,header:bold".to_string(),
        "--pointer=▌".to_string(),
        "--delimiter=\t".to_string(),
        "--with-nth=2..".to_string(),
    ];
    if let Some(query) = initial_text {
        args.push(format!("--query={}", query));
    }
    if let Some(cursor) = starting_cursor {
        args.push(format!("--bind=load:pos({})", cursor + 1));
    }
    if header_lines > 0 {
        args.push(format!("--header-lines={}", header_lines));
    }
    builder.custom_args(args);

    builder
        .build()
        .expect("fzf builder should always succeed with default options")
}

/// Formats items as `"{idx}\t{display}"` for passing to fzf.
///
/// The index prefix lets us recover the original position from fzf's output
/// without relying on string matching, which breaks when multiple items have
/// the same display string.
pub(crate) fn indexed_items(display_options: &[String]) -> Vec<String> {
    display_options
        .iter()
        .enumerate()
        .map(|(i, d)| format!("{}\t{}", i, d))
        .collect()
}

/// Parses the index from a line returned by fzf when items were formatted with
/// `indexed_items`. Returns `None` if the line is malformed.
pub(crate) fn parse_fzf_index(line: &str) -> Option<usize> {
    line.split('\t').next()?.trim().parse().ok()
}

impl<T: 'static> SelectBuilder<T> {
    /// Set starting cursor position.
    pub fn with_starting_cursor(mut self, cursor: usize) -> Self {
        self.starting_cursor = Some(cursor);
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
    /// When set to `n`, the first `n` items are displayed as a fixed header
    /// that is always visible but cannot be selected. Mirrors fzf's
    /// `--header-lines` flag, matching the shell plugin's porcelain output
    /// where the first line contains column headings.
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
    /// Returns an error if the fzf process fails to start or interact.
    pub fn prompt(self) -> Result<Option<T>>
    where
        T: std::fmt::Display + Clone,
    {
        // Bail immediately when stdin is not a terminal to prevent the process
        // from blocking indefinitely on a detached or non-interactive session.
        if !std::io::stdin().is_terminal() {
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

        let fzf = build_fzf(
            &self.message,
            self.help_message,
            self.initial_text.as_deref(),
            self.starting_cursor,
            self.header_lines,
        );

        let selected = run_with_output(fzf, indexed_items(&display_options));

        match selected {
            None => Ok(None),
            Some(selection) if selection.trim().is_empty() => Ok(None),
            Some(selection) => {
                Ok(parse_fzf_index(&selection).and_then(|index| self.options.get(index).cloned()))
            }
        }
    }
}

/// Runs a yes/no confirmation prompt via fzf.
///
/// Returns `Ok(Some(true))` for Yes, `Ok(Some(false))` for No, and `Ok(None)`
/// if cancelled.
fn prompt_confirm(message: &str, default: Option<bool>) -> Result<Option<bool>> {
    let items = ["Yes", "No"];
    let starting_cursor = if default == Some(false) {
        Some(1)
    } else {
        Some(0)
    };

    let fzf = build_fzf(message, None, None, starting_cursor, 0);
    let selected = run_with_output(fzf, items.iter().copied());

    let result: Option<bool> = match selected.as_deref().map(str::trim) {
        Some("Yes") => Some(true),
        Some("No") => Some(false),
        _ => None,
    };

    Ok(result)
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
    fn test_indexed_items() {
        let fixture = vec![
            "Apple".to_string(),
            "Apple".to_string(),
            "Banana".to_string(),
        ];
        let actual = indexed_items(&fixture);
        let expected = vec!["0\tApple", "1\tApple", "2\tBanana"];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_parse_fzf_index() {
        assert_eq!(parse_fzf_index("0\tApple"), Some(0));
        assert_eq!(parse_fzf_index("2\tBanana"), Some(2));
        assert_eq!(parse_fzf_index("1\tApple"), Some(1));
        assert_eq!(parse_fzf_index("notanindex\tApple"), None);
        assert_eq!(parse_fzf_index(""), None);
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
