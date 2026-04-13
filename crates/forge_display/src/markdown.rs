use std::sync::OnceLock;

use derive_setters::Setters;
use regex::Regex;
use termimad::crossterm::style::{Attribute, Color};
use termimad::{CompoundStyle, LineStyle, MadSkin};

use crate::code::{CodeBlockParser, SyntaxHighlighter};

/// MarkdownFormat provides functionality for formatting markdown text for
/// terminal display.
#[derive(Clone, Setters)]
#[setters(into, strip_option)]
pub struct MarkdownFormat {
    skin: MadSkin,
    max_consecutive_newlines: usize,
    #[setters(skip)]
    highlighter: OnceLock<SyntaxHighlighter>,
}

impl Default for MarkdownFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownFormat {
    /// Create a new MarkdownFormat with the default skin
    pub fn new() -> Self {
        let mut skin = MadSkin::default();
        let compound_style = CompoundStyle::new(Some(Color::Cyan), None, Default::default());
        skin.inline_code = compound_style.clone();

        let codeblock_style = CompoundStyle::new(None, None, Default::default());
        skin.code_block = LineStyle::new(codeblock_style, Default::default());

        let mut strikethrough_style = CompoundStyle::with_attr(Attribute::CrossedOut);
        strikethrough_style.add_attr(Attribute::Dim);
        skin.strikeout = strikethrough_style;

        Self {
            skin,
            max_consecutive_newlines: 2,
            highlighter: OnceLock::new(),
        }
    }

    /// Render the markdown content to a string formatted for terminal display.
    pub fn render(&self, content: impl Into<String>) -> String {
        let content = self.strip_excessive_newlines(content.into().trim());
        if content.is_empty() {
            return String::new();
        }

        // Extract code blocks
        let processed = CodeBlockParser::new(&content);

        // Render with termimad, then restore highlighted code
        let rendered = self.skin.term_text(processed.markdown()).to_string();
        let highlighter = self.highlighter.get_or_init(SyntaxHighlighter::default);
        processed.restore(highlighter, rendered).trim().to_string()
    }

    fn strip_excessive_newlines(&self, content: &str) -> String {
        if content.is_empty() {
            return String::new();
        }
        Regex::new(&format!(r"\n{{{},}}", self.max_consecutive_newlines + 1))
            .unwrap()
            .replace_all(content, "\n".repeat(self.max_consecutive_newlines))
            .into()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_render_simple_markdown() {
        let fixture = "# Test Heading\nThis is a test.";
        let markdown = MarkdownFormat::new();
        let actual = markdown.render(fixture);

        // Basic verification that output is non-empty
        assert!(!actual.is_empty());
    }

    #[test]
    fn test_render_empty_markdown() {
        let fixture = "";
        let markdown = MarkdownFormat::new();
        let actual = markdown.render(fixture);

        // Verify empty input produces empty output
        assert!(actual.is_empty());
    }

    #[test]
    fn test_strip_excessive_newlines_default() {
        let fixture = "Line 1\n\n\n\nLine 2";
        let formatter = MarkdownFormat::new();
        let actual = formatter.strip_excessive_newlines(fixture);
        let expected = "Line 1\n\nLine 2";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_strip_excessive_newlines_custom() {
        let fixture = "Line 1\n\n\n\nLine 2";
        let formatter = MarkdownFormat::new().max_consecutive_newlines(3_usize);
        let actual = formatter.strip_excessive_newlines(fixture);
        let expected = "Line 1\n\n\nLine 2";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_render_with_excessive_newlines() {
        let fixture = "# Heading\n\n\n\nParagraph";
        let markdown = MarkdownFormat::new();

        // Use the default max_consecutive_newlines (2)
        let actual = markdown.render(fixture);

        // Compare with expected content containing only 2 newlines
        let expected = markdown.render("# Heading\n\nParagraph");

        // Strip any ANSI codes and whitespace for comparison
        let actual_clean = strip_ansi_escapes::strip_str(&actual).trim().to_string();
        let expected_clean = strip_ansi_escapes::strip_str(&expected).trim().to_string();

        assert_eq!(actual_clean, expected_clean);
    }

    #[test]
    fn test_render_with_custom_max_newlines() {
        let fixture = "# Heading\n\n\n\nParagraph";
        let markdown = MarkdownFormat::new().max_consecutive_newlines(1_usize);

        // Use a custom max_consecutive_newlines (1)
        let actual = markdown.render(fixture);

        // Compare with expected content containing only 1 newline
        let expected = markdown.render("# Heading\nParagraph");

        // Strip any ANSI codes and whitespace for comparison
        let actual_clean = strip_ansi_escapes::strip_str(&actual).trim().to_string();
        let expected_clean = strip_ansi_escapes::strip_str(&expected).trim().to_string();

        assert_eq!(actual_clean, expected_clean);
    }

    #[test]
    fn test_highlight_code_block() {
        let md = MarkdownFormat::new();
        let actual = md.render("```rust\nfn main() {}\n```");
        assert!(actual.contains("\x1b[")); // Contains ANSI escape codes
        assert!(strip_ansi_escapes::strip_str(&actual).contains("fn main()"));
    }
}
