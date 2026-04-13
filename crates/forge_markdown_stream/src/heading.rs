//! Heading rendering with theme-based styling.

use streamdown_render::simple_wrap;

use crate::inline::render_inline_content;
use crate::style::{HeadingStyler, InlineStyler};

/// Render a heading with appropriate styling.
pub fn render_heading<S: InlineStyler + HeadingStyler>(
    level: u8,
    content: &str,
    width: usize,
    margin: &str,
    styler: &S,
) -> Vec<String> {
    // Create the heading prefix (e.g., "# ", "## ", etc.)
    let prefix = "#".repeat(level as usize);

    // For h1, uppercase the content before rendering inline elements
    let content_to_render = if level == 1 {
        content.to_uppercase()
    } else {
        content.to_string()
    };

    // First render inline elements (bold, italic, etc.) in the content
    let rendered_content = render_inline_content(&content_to_render, styler);

    // Adjust width to account for the prefix (e.g., "# " = 2 chars, "## " = 3
    // chars, etc.)
    let prefix_display_width = level as usize + 1;
    let content_width = width.saturating_sub(prefix_display_width);
    let lines = simple_wrap(&rendered_content, content_width);
    let mut result = Vec::new();

    for line in lines {
        let formatted = match level {
            1 => {
                // H1: Bold, left-aligned, uppercase, with dimmed prefix
                format!(
                    "{}\n{}{} {}",
                    margin,
                    margin,
                    styler.dimmed(&styler.h1(&prefix)),
                    styler.h1(&line)
                )
            }
            2 => {
                // H2: Bold, bright color, left-aligned, with dimmed prefix
                format!(
                    "{}\n{}{} {}",
                    margin,
                    margin,
                    styler.dimmed(&styler.h2(&prefix)),
                    styler.h2(&line)
                )
            }
            3 => {
                format!(
                    "{}{} {}",
                    margin,
                    styler.dimmed(&styler.h3(&prefix)),
                    styler.h3(&line)
                )
            }
            4 => {
                format!(
                    "{}{} {}",
                    margin,
                    styler.dimmed(&styler.h4(&prefix)),
                    styler.h4(&line)
                )
            }
            5 => {
                format!(
                    "{}{} {}",
                    margin,
                    styler.dimmed(&styler.h5(&prefix)),
                    styler.h5(&line)
                )
            }
            _ => {
                format!(
                    "{}{} {}",
                    margin,
                    styler.dimmed(&styler.h6(&prefix)),
                    styler.h6(&line)
                )
            }
        };
        result.push(formatted);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::TagStyler;

    fn render(level: u8, content: &str) -> String {
        render_heading(level, content, 80, "  ", &TagStyler).join("\n")
    }

    fn render_with_width(level: u8, content: &str, width: usize) -> String {
        render_heading(level, content, width, "  ", &TagStyler).join("\n")
    }

    fn render_with_margin(level: u8, content: &str, margin: &str) -> String {
        render_heading(level, content, 80, margin, &TagStyler).join("\n")
    }

    #[test]
    fn test_h1_simple() {
        insta::assert_snapshot!(render(1, "Hello World"), @"

        <dim><h1>#</h1></dim> <h1>HELLO WORLD</h1>
        ");
    }

    #[test]
    fn test_h2_simple() {
        insta::assert_snapshot!(render(2, "Chapter One"), @"

        <dim><h2>##</h2></dim> <h2>Chapter One</h2>
        ");
    }

    #[test]
    fn test_h3_simple() {
        insta::assert_snapshot!(render(3, "Section Title"), @"  <dim><h3>###</h3></dim> <h3>Section Title</h3>");
    }

    #[test]
    fn test_h4_simple() {
        insta::assert_snapshot!(render(4, "Subsection"), @"  <dim><h4>####</h4></dim> <h4>Subsection</h4>");
    }

    #[test]
    fn test_h5_simple() {
        insta::assert_snapshot!(render(5, "Minor Heading"), @"  <dim><h5>#####</h5></dim> <h5>Minor Heading</h5>");
    }

    #[test]
    fn test_h6_simple() {
        insta::assert_snapshot!(render(6, "Smallest Heading"), @"  <dim><h6>######</h6></dim> <h6>Smallest Heading</h6>");
    }

    #[test]
    fn test_h1_with_inline_bold() {
        insta::assert_snapshot!(render(1, "Hello **bold** world"), @"

        <dim><h1>#</h1></dim> <h1>HELLO <b>BOLD</b> WORLD</h1>
        ");
    }

    #[test]
    fn test_h2_with_inline_italic() {
        insta::assert_snapshot!(render(2, "Hello *italic* text"), @"

        <dim><h2>##</h2></dim> <h2>Hello <i>italic</i> text</h2>
        ");
    }

    #[test]
    fn test_h3_with_code() {
        insta::assert_snapshot!(render(3, "Using `code` here"), @"  <dim><h3>###</h3></dim> <h3>Using <code>code</code> here</h3>");
    }

    #[test]
    fn test_heading_level_beyond_6() {
        // Level 7+ should fall through to h6 styling
        insta::assert_snapshot!(render(7, "Level Seven"), @"  <dim><h6>#######</h6></dim> <h6>Level Seven</h6>");
        insta::assert_snapshot!(render(10, "Level Ten"), @"  <dim><h6>##########</h6></dim> <h6>Level Ten</h6>");
    }

    #[test]
    fn test_empty_content() {
        insta::assert_snapshot!(render(1, ""), @"

        <dim><h1>#</h1></dim> <h1></h1>
        ");
    }

    #[test]
    fn test_custom_margin() {
        insta::assert_snapshot!(render_with_margin(1, "Title", "    "), @"

        <dim><h1>#</h1></dim> <h1>TITLE</h1>
        ");
        insta::assert_snapshot!(render_with_margin(3, "Section", ">>> "), @">>> <dim><h3>###</h3></dim> <h3>Section</h3>");
    }

    #[test]
    fn test_no_margin() {
        insta::assert_snapshot!(render_with_margin(1, "Title", ""), @"

        <dim><h1>#</h1></dim> <h1>TITLE</h1>
        ");
        insta::assert_snapshot!(render_with_margin(3, "Section", ""), @"<dim><h3>###</h3></dim> <h3>Section</h3>");
    }

    #[test]
    fn test_wrapping_narrow_width() {
        insta::assert_snapshot!(render_with_width(1, "This is a very long heading that should wrap", 20), @"

        <dim><h1>#</h1></dim> <h1>THIS IS A VERY</h1>

        <dim><h1>#</h1></dim> <h1>LONG HEADING THAT</h1>

        <dim><h1>#</h1></dim> <h1>SHOULD WRAP</h1>
        ");
    }

    #[test]
    fn test_h3_wrapping() {
        insta::assert_snapshot!(render_with_width(3, "A long section title that wraps", 15), @"
        <dim><h3>###</h3></dim> <h3>A long</h3>
        <dim><h3>###</h3></dim> <h3>section</h3>
        <dim><h3>###</h3></dim> <h3>title that</h3>
        <dim><h3>###</h3></dim> <h3>wraps</h3>
        ");
    }

    #[test]
    fn test_special_characters() {
        insta::assert_snapshot!(render(2, "Hello & Goodbye < World >"), @"

        <dim><h2>##</h2></dim> <h2>Hello & Goodbye < World ></h2>
        ");
    }

    #[test]
    fn test_heading_with_link() {
        insta::assert_snapshot!(render(3, "See [documentation](https://example.com)"), @r#"  <dim><h3>###</h3></dim> <h3>See <a href="https://example.com">documentation</a></h3>"#);
    }

    #[test]
    fn test_mixed_inline_styles() {
        insta::assert_snapshot!(render(2, "**Bold** and *italic* and `code`"), @"

        <dim><h2>##</h2></dim> <h2><b>Bold</b> and <i>italic</i> and <code>code</code></h2>
        ");
    }

    #[test]
    fn test_all_levels_structure() {
        // H1 and H2 have extra newline prefix
        let h1 = render(1, "H1");
        let h2 = render(2, "H2");
        let h3 = render(3, "H3");

        assert!(h1.contains("\n"), "H1 should have newline");
        assert!(h2.contains("\n"), "H2 should have newline");
        assert!(!h3.starts_with("\n"), "H3 should not start with newline");
    }
}
