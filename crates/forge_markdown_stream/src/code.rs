//! Code block rendering with syntax highlighting and line wrapping.

use forge_syntax::{Theme, code_wrap, highlight_line};

use crate::utils::{ThemeMode, detect_theme_mode};

const RESET: &str = "\x1b[0m";

/// Code block highlighter using the internal ANSI lexer.
pub struct CodeHighlighter {
    theme: Theme,
}

impl Default for CodeHighlighter {
    fn default() -> Self {
        Self::new(match detect_theme_mode() {
            ThemeMode::Dark => Theme::Dark,
            ThemeMode::Light => Theme::Light,
        })
    }
}

impl CodeHighlighter {
    /// Creates a highlighter using the caller-selected terminal palette.
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    /// Replaces the terminal palette used for subsequent code lines.
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    /// Highlight a single line of code.
    fn highlight_line(&self, line: &str, language: Option<&str>) -> String {
        highlight_line(line, language, self.theme)
    }

    /// Render a code line with margin, wrapping if needed.
    ///
    /// Returns multiple lines if the code exceeds the available width.
    pub fn render_code_line(
        &self,
        line: &str,
        language: Option<&str>,
        margin: &str,
        width: usize,
    ) -> Vec<String> {
        // Use code_wrap with pretty_broken=true for line wrapping
        let (indent, wrapped_lines) = code_wrap(line, width, true);

        let mut result = Vec::new();

        for (i, code_line) in wrapped_lines.iter().enumerate() {
            let highlighted = self.highlight_line(code_line, language);

            // Add continuation indent for wrapped lines
            let line_indent = if i == 0 {
                ""
            } else {
                &"  ".repeat(indent.min(4) / 2 + 1)
            };

            result.push(format!("{}{}{}{}", margin, line_indent, highlighted, RESET));
        }

        if result.is_empty() {
            result.push(format!("{}{}", margin, RESET));
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::{CodeHighlighter, code_wrap};

    #[test]
    fn test_code_wrap_short_line() {
        let (indent, lines) = code_wrap("let x = 1;", 80, true);
        assert_eq!(indent, 0);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "let x = 1;");
    }

    #[test]
    fn test_code_wrap_with_indent() {
        let (indent, lines) = code_wrap("    let x = 1;", 80, true);
        assert_eq!(indent, 4);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_code_wrap_long_line() {
        let long_line = "x".repeat(100);
        let (_, lines) = code_wrap(&long_line, 40, true);
        assert!(lines.len() > 1);
    }

    #[test]
    fn test_code_wrap_empty() {
        let (indent, lines) = code_wrap("", 80, true);
        assert_eq!(indent, 0);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_code_wrap_preserves_indented_unicode_text() {
        let fixture = "  abcdef\u{ac00}\u{b098}\u{b2e4}";
        let (actual_indent, actual_lines) = code_wrap(fixture, 8, true);
        let expected_indent = 2;
        let expected_lines = vec!["  ab", "cd", "ef", "\u{ac00}", "\u{b098}", "\u{b2e4}"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        assert_eq!(actual_indent, expected_indent);
        assert_eq!(actual_lines, expected_lines);
    }

    #[test]
    fn test_unknown_language_line_has_only_terminal_reset() {
        let fixture = CodeHighlighter::default();
        let actual = fixture.render_code_line("launch --safe", Some("not-a-language"), "", 80);
        let expected = vec!["launch --safe\x1b[0m".to_string()];
        assert_eq!(actual, expected);
    }
}
