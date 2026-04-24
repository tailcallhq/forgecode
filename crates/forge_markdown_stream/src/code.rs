//! Code block rendering with syntax highlighting and line wrapping.

use streamdown_render::code::code_wrap;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::as_24_bit_terminal_escaped;

use crate::utils::{ThemeMode, detect_theme_mode};

const RESET: &str = "\x1b[0m";

/// Code block highlighter using syntect.
pub struct CodeHighlighter {
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
    theme_mode: ThemeMode,
}

impl Default for CodeHighlighter {
    fn default() -> Self {
        Self {
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
            theme_mode: detect_theme_mode(),
        }
    }
}

impl CodeHighlighter {
    /// Highlight a single line of code.
    fn highlight_line(&self, line: &str, language: Option<&str>) -> String {
        let syntax = language
            .and_then(|lang| self.syntax_set.find_syntax_by_token(lang))
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());

        let theme_name = match self.theme_mode {
            ThemeMode::Dark => "base16-ocean.dark",
            ThemeMode::Light => "InspiredGitHub",
        };
        let theme = self.theme_set.themes.get(theme_name).unwrap_or_else(|| {
            // Fallback to base16-ocean.dark if theme not found
            self.theme_set
                .themes
                .get("base16-ocean.dark")
                .expect("Default theme should exist")
        });
        let mut highlighter = HighlightLines::new(syntax, theme);

        match highlighter.highlight_line(line, &self.syntax_set) {
            Ok(ranges) => as_24_bit_terminal_escaped(&ranges[..], false),
            Err(_) => line.to_string(),
        }
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
    use streamdown_render::code::code_wrap;

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
}
