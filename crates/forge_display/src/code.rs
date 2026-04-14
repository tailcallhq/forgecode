use std::sync::Arc;

use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::as_24_bit_terminal_escaped;
use terminal_colorsaurus::{QueryOptions, ThemeMode, theme_mode};
use two_face::theme::EmbeddedThemeName;

/// Loads and caches syntax highlighting resources.
#[derive(Clone)]
pub struct SyntaxHighlighter {
    syntax_set: Arc<SyntaxSet>,
    theme_set: Arc<ThemeSet>,
}

impl Default for SyntaxHighlighter {
    fn default() -> Self {
        // Use two-face's extended syntax set which includes TOML, Rust, Python, etc.
        Self {
            syntax_set: Arc::new(two_face::syntax::extra_newlines()),
            theme_set: Arc::new(two_face::theme::extra().into()),
        }
    }
}

impl SyntaxHighlighter {
    /// Detects whether the terminal is using a dark or light background.
    fn is_dark_theme() -> bool {
        match theme_mode(QueryOptions::default()) {
            Ok(ThemeMode::Light) => false,
            Ok(ThemeMode::Dark) | Err(_) => true,
        }
    }

    /// Syntax-highlights `code` for the given language token (e.g. `"toml"`,
    /// `"rust"`), returning an ANSI-escaped string ready for terminal output.
    ///
    /// The theme is chosen automatically based on the terminal background
    /// (dark → `base16-ocean.dark`, light → `InspiredGitHub`). Falls back to
    /// plain text if the language is unrecognised.
    pub fn highlight(&self, code: &str, lang: &str) -> String {
        let syntax = self
            .syntax_set
            .find_syntax_by_token(lang)
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());
        let theme_name = if Self::is_dark_theme() {
            EmbeddedThemeName::Base16OceanDark
        } else {
            EmbeddedThemeName::InspiredGithub
        };
        let theme = &self.theme_set.themes[theme_name.as_name()];
        let mut hl = HighlightLines::new(syntax, theme);

        code.lines()
            .filter_map(|line| hl.highlight_line(line, &self.syntax_set).ok())
            .map(|ranges| format!("{}\x1b[0m", as_24_bit_terminal_escaped(&ranges, false)))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// A code block extracted from markdown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodeBlock {
    code: String,
    lang: String,
}

/// Holds extracted code blocks and processed markdown with placeholders.
#[derive(Clone)]
pub struct CodeBlockParser {
    markdown: String,
    blocks: Vec<CodeBlock>,
}

impl CodeBlockParser {
    /// Extract code blocks from markdown content.
    /// Supports both standard and indented code blocks (up to 3 spaces of
    /// indentation).
    pub fn new(content: &str) -> Self {
        let original_lines: Vec<&str> = content.lines().collect();
        let mut blocks = Vec::new();
        let mut result = String::new();
        let mut in_code = false;
        let mut code_lines: Vec<&str> = Vec::new();
        let mut lang = String::new();

        for line in &original_lines {
            // Check if line is a code fence (with or without indentation)
            if let Some(fence_lang) = Self::detect_code_fence(line) {
                if !in_code {
                    // Opening fence
                    lang = fence_lang;
                    in_code = true;
                } else {
                    // Closing fence
                    result.push_str(&format!("\x00{}\x00\n", blocks.len()));
                    blocks.push(CodeBlock { code: code_lines.join("\n"), lang: lang.clone() });
                    code_lines.clear();
                    in_code = false;
                }
            } else if in_code {
                // Inside code block - collect lines
                code_lines.push(line);
            } else {
                // Regular markdown line
                result.push_str(line);
                result.push('\n');
            }
        }

        Self { markdown: result, blocks }
    }

    /// Detect if a line is a code fence marker (```).
    /// Returns Some(language) if it's an opening fence with a language tag,
    /// Some("") if it's a fence without a language tag (opening or closing),
    /// None if it's not a code fence.
    fn detect_code_fence(line: &str) -> Option<String> {
        let trimmed = line.trim_start();
        if let Some(stripped) = trimmed.strip_prefix("```") {
            // Extract language tag (everything after ``` until whitespace or end)
            let lang = stripped.split_whitespace().next().unwrap_or("");
            Some(lang.to_string())
        } else {
            None
        }
    }

    /// Get the processed markdown with placeholders.
    pub fn markdown(&self) -> &str {
        &self.markdown
    }

    /// Get the extracted code blocks.
    #[cfg(test)]
    pub(crate) fn blocks(&self) -> &[CodeBlock] {
        &self.blocks
    }

    /// Replace placeholders with highlighted code blocks.
    pub fn restore(&self, highlighter: &SyntaxHighlighter, mut rendered: String) -> String {
        for (i, block) in self.blocks.iter().enumerate() {
            let highlighted = highlighter.highlight(&block.code, &block.lang);
            rendered = rendered.replace(&format!("\x00{i}\x00"), &highlighted);
        }
        rendered
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn strip_ansi(s: &str) -> String {
        strip_ansi_escapes::strip_str(s).to_string()
    }

    fn fixture_parser(name: &str) -> CodeBlockParser {
        let content = match name {
            "code-01" => include_str!("fixtures/code-01.md"),
            "code-02" => include_str!("fixtures/code-02.md"),
            _ => panic!("Unknown fixture: {}", name),
        };
        CodeBlockParser::new(content)
    }

    #[test]
    fn test_no_code_blocks() {
        let fixture = "Hello world\nThis is plain text.";
        let parser = CodeBlockParser::new(fixture);

        let actual = parser.blocks().len();
        let expected = 0;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_single_code_block() {
        let fixture = "```rust\nfn main() {}\n```";
        let parser = CodeBlockParser::new(fixture);

        let actual = parser.blocks().len();
        let expected = 1;

        assert_eq!(actual, expected);
        assert_eq!(parser.blocks()[0].lang, "rust");
        assert_eq!(parser.blocks()[0].code, "fn main() {}");
    }

    #[test]
    fn test_preserves_indentation_inside_code_block() {
        let fixture = "```rust\n    let x = 1;\n```";
        let parser = CodeBlockParser::new(fixture);

        let actual = &parser.blocks()[0].code;
        let expected = "    let x = 1;";

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_detects_indented_code_fence() {
        let fixture = "1. Item\n\n   ```rust\n   code\n   ```";
        let parser = CodeBlockParser::new(fixture);

        let actual = parser.blocks().len();
        let expected = 1;

        assert_eq!(actual, expected);
        assert_eq!(parser.blocks()[0].lang, "rust");
    }

    #[test]
    fn test_multiple_languages() {
        let fixture = "```rust\nrust code\n```\n\n```python\npython code\n```";
        let parser = CodeBlockParser::new(fixture);

        let actual = parser.blocks().len();
        let expected = 2;

        assert_eq!(actual, expected);
        assert_eq!(parser.blocks()[0].lang, "rust");
        assert_eq!(parser.blocks()[1].lang, "python");
    }

    #[test]
    fn test_extracts_indented_code_blocks_from_fixture() {
        let parser = fixture_parser("code-01");

        let actual = parser.blocks().len();
        let expected = 4;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extracts_standard_code_blocks_from_fixture() {
        let parser = fixture_parser("code-02");

        let actual = parser.blocks().len();
        let expected = 3;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_restore_replaces_placeholders_with_highlighted_code() {
        let fixture = "```rust\ncode\n```";
        let highlighter = SyntaxHighlighter::default();
        let parser = CodeBlockParser::new(fixture);

        let actual = strip_ansi(&parser.restore(&highlighter, parser.markdown().to_string()));

        assert!(actual.contains("code"));
    }

    #[test]
    fn test_full_extraction_and_restoration_flow() {
        let fixture = "Hi\n```rust\nlet x = 1;\n```\nBye";
        let highlighter = SyntaxHighlighter::default();
        let parser = CodeBlockParser::new(fixture);

        let actual = strip_ansi(&parser.restore(&highlighter, parser.markdown().to_string()));

        assert!(actual.contains("Hi"));
        assert!(actual.contains("let x = 1"));
        assert!(actual.contains("Bye"));
    }

    #[test]
    fn test_highlighter_can_be_reused() {
        let highlighter = SyntaxHighlighter::default();

        let parser1 = CodeBlockParser::new("```rust\nlet x = 1;\n```");
        let parser2 = CodeBlockParser::new("```python\nprint('hello')\n```");

        let actual1 = strip_ansi(&parser1.restore(&highlighter, parser1.markdown().to_string()));
        let actual2 = strip_ansi(&parser2.restore(&highlighter, parser2.markdown().to_string()));

        assert!(actual1.contains("let x = 1"));
        assert!(actual2.contains("print('hello')"));
    }
}
