//! Inline content rendering with theme-based formatting.

use streamdown_parser::{InlineElement, InlineParser};

use crate::style::InlineStyler;

/// Render inline elements to a string using a styler.
pub fn render_inline_content<S: InlineStyler>(content: &str, styler: &S) -> String {
    render_inline_elements(&InlineParser::new().parse(content), styler)
}

/// Render inline elements to a string using a styler.
pub fn render_inline_elements<S: InlineStyler>(elements: &[InlineElement], styler: &S) -> String {
    let mut result = String::new();
    for element in elements {
        match element {
            InlineElement::Text(text) => {
                result.push_str(&styler.text(text));
            }
            InlineElement::Bold(text) => {
                result.push_str(&styler.bold(text));
            }
            InlineElement::Italic(text) => {
                result.push_str(&styler.italic(text));
            }
            InlineElement::BoldItalic(text) => {
                result.push_str(&styler.bold_italic(text));
            }
            InlineElement::Strikeout(text) => {
                result.push_str(&styler.strikethrough(text));
            }
            InlineElement::Underline(text) => {
                result.push_str(&styler.underline(text));
            }
            InlineElement::Code(text) => {
                result.push_str(&styler.code(text));
            }
            InlineElement::Link { text, url } => {
                result.push_str(&styler.link(text, url));
            }
            InlineElement::Image { alt, url } => {
                result.push_str(&styler.image(alt, url));
            }
            InlineElement::Footnote(text) => {
                result.push_str(&styler.footnote(text));
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::TagStyler;

    fn render(content: &str) -> String {
        render_inline_content(content, &TagStyler)
    }

    #[test]
    fn test_plain_text() {
        insta::assert_snapshot!(render("hello world"), @"hello world");
    }

    #[test]
    fn test_html_entities() {
        insta::assert_snapshot!(render("&amp; &lt; &gt; &quot;"), @r###"& < > ""###);
    }

    #[test]
    fn test_bold() {
        insta::assert_snapshot!(render("**bold**"), @"<b>bold</b>");
    }

    #[test]
    fn test_italic() {
        insta::assert_snapshot!(render("*italic*"), @"<i>italic</i>");
    }

    #[test]
    fn test_bold_italic() {
        insta::assert_snapshot!(render("***text***"), @"<b><i>text</i></b>");
    }

    #[test]
    fn test_strikethrough() {
        insta::assert_snapshot!(render("~~struck~~"), @"<s>struck</s>");
    }

    #[test]
    fn test_code() {
        insta::assert_snapshot!(render("`code`"), @"<code>code</code>");
    }

    #[test]
    fn test_underline() {
        insta::assert_snapshot!(render("__underline__"), @"<u>underline</u>");
    }

    #[test]
    fn test_link() {
        insta::assert_snapshot!(render("[click](https://example.com)"), @r###"<a href="https://example.com">click</a>"###);
    }

    #[test]
    fn test_image() {
        insta::assert_snapshot!(render("![alt](image.png)"), @r###"<img alt="alt" src="image.png"/>"###);
    }

    #[test]
    fn test_mixed() {
        insta::assert_snapshot!(render("hello **bold** and *italic*"), @"hello <b>bold</b> and <i>italic</i>");
    }

    #[test]
    fn test_multiple_bold() {
        insta::assert_snapshot!(render("**one** and **two**"), @"<b>one</b> and <b>two</b>");
    }

    #[test]
    fn test_entities_in_bold() {
        insta::assert_snapshot!(render("**&amp;**"), @"<b>&</b>");
    }

    #[test]
    fn test_entities_in_link() {
        insta::assert_snapshot!(render("[&lt;click&gt;](https://example.com)"), @r###"<a href="https://example.com"><click></a>"###);
    }

    #[test]
    fn test_code_content() {
        insta::assert_snapshot!(render("`let x = 1;`"), @"<code>let x = 1;</code>");
    }

    #[test]
    fn test_link_special_url() {
        insta::assert_snapshot!(render("[link](https://example.com/path?q=1&b=2)"), @r###"<a href="https://example.com/path?q=1&b=2">link</a>"###);
    }

    #[test]
    fn test_empty() {
        insta::assert_snapshot!(render(""), @"");
    }

    #[test]
    fn test_whitespace() {
        insta::assert_snapshot!(render("hello   world"), @"hello   world");
    }

    #[test]
    fn test_image_empty_alt() {
        insta::assert_snapshot!(render("![](image.png)"), @r###"<img alt="" src="image.png"/>"###);
    }
}
