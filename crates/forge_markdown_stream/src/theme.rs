//! Theme configuration for markdown rendering.
//!
//! Provides customizable styling for all markdown elements using the `colored`
//! crate.

use colored::{Color, ColoredString, Colorize};
use streamdown_parser::decode_html_entities;

use crate::style::{HeadingStyler, InlineStyler, ListStyler, TableStyler};

/// Style configuration for a single element.
#[derive(Clone, Debug, Default)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub dimmed: bool,
}

impl Style {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fg(mut self, color: Color) -> Self {
        self.fg = Some(color);
        self
    }

    pub fn bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    pub fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }

    pub fn dimmed(mut self) -> Self {
        self.dimmed = true;
        self
    }

    /// Returns the raw ANSI escape sequence for this style (no text, no reset).
    pub fn escapes(&self) -> String {
        use crate::utils::{bg_code, fg_code};
        let mut s = String::new();
        if let Some(fg) = self.fg {
            s.push_str(&fg_code(fg));
        }
        if let Some(bg) = self.bg {
            s.push_str(&bg_code(bg));
        }
        if self.bold {
            s.push_str("\x1b[1m");
        }
        if self.italic {
            s.push_str("\x1b[3m");
        }
        if self.underline {
            s.push_str("\x1b[4m");
        }
        if self.strikethrough {
            s.push_str("\x1b[9m");
        }
        if self.dimmed {
            s.push_str("\x1b[2m");
        }
        s
    }

    /// Apply this style to a string.
    pub fn apply(&self, text: &str) -> ColoredString {
        let mut result = text.normal();

        if let Some(fg) = self.fg {
            result = result.color(fg);
        }
        if let Some(bg) = self.bg {
            result = result.on_color(bg);
        }
        if self.bold {
            result = result.bold();
        }
        if self.italic {
            result = result.italic();
        }
        if self.underline {
            result = result.underline();
        }
        if self.strikethrough {
            result = result.strikethrough();
        }
        if self.dimmed {
            result = result.dimmed();
        }

        result
    }
}

/// Theme containing styles for all markdown elements.
#[derive(Clone, Debug)]
pub struct Theme {
    // Inline styles
    pub bold: Style,
    pub italic: Style,
    pub code: Style,
    pub strikethrough: Style,
    pub link: Style,
    pub link_url: Style,

    // Block styles
    pub heading1: Style,
    pub heading2: Style,
    pub heading3: Style,
    pub heading4: Style,
    pub heading5: Style,
    pub heading6: Style,

    // List styles
    pub bullet_dash: Style,
    pub bullet_asterisk: Style,
    pub bullet_plus: Style,
    pub bullet_plus_expand: Style,
    pub list_number: Style,
    pub checkbox_checked: Style,
    pub checkbox_unchecked: Style,

    // Table styles
    pub table_header: Style,
    pub table_border: Style,
    pub table_cell: Style,

    // Quote/Think styles
    pub blockquote: Style,
    pub blockquote_border: Style,
    pub think: Style,
    pub think_border: Style,

    // Code block
    pub code_block_lang: Style,

    // Horizontal rule
    pub hr: Style,

    // Mermaid diagram styles
    pub mermaid_node: Style,
    pub mermaid_edge: Style,
    pub mermaid_label: Style,
    pub mermaid_border: Style,
    pub mermaid_arrow_head: Style,
    pub mermaid_node_start: Style,
    pub mermaid_node_end: Style,
    pub mermaid_node_process: Style,
    pub mermaid_node_decision: Style,
    pub mermaid_edge_yes: Style,
    pub mermaid_edge_no: Style,
    pub mermaid_bg: Style,
}
impl Default for Theme {
    fn default() -> Self {
        Self::detect()
    }
}

/// Theme-based styler that outputs ANSI codes.
impl InlineStyler for Theme {
    fn text(&self, text: &str) -> String {
        decode_html_entities(text)
    }

    fn bold(&self, text: &str) -> String {
        self.bold.apply(&decode_html_entities(text)).to_string()
    }

    fn italic(&self, text: &str) -> String {
        self.italic.apply(&decode_html_entities(text)).to_string()
    }

    fn bold_italic(&self, text: &str) -> String {
        let decoded = decode_html_entities(text);
        let styled = self.bold.apply(&decoded);
        self.italic.apply(&styled.to_string()).to_string()
    }

    fn strikethrough(&self, text: &str) -> String {
        self.strikethrough
            .apply(&decode_html_entities(text))
            .to_string()
    }

    fn underline(&self, text: &str) -> String {
        format!("\x1b[4m{}\x1b[24m", decode_html_entities(text))
    }

    fn code(&self, text: &str) -> String {
        self.code.apply(text).to_string()
    }

    fn link(&self, text: &str, url: &str) -> String {
        let mut result = String::new();
        result.push_str("\x1b]8;;");
        result.push_str(url);
        result.push_str("\x1b\\");
        result.push_str(&self.link.apply(&decode_html_entities(text)).to_string());
        result.push_str("\x1b]8;;\x1b\\");
        result.push(' ');
        result.push_str(&self.link_url.apply(&format!("({})", url)).to_string());
        result
    }

    fn image(&self, alt: &str, _url: &str) -> String {
        format!("[🖼 {}]", alt)
    }

    fn footnote(&self, text: &str) -> String {
        text.to_string()
    }

    fn dimmed(&self, text: &str) -> String {
        Style::new().dimmed().apply(text).to_string()
    }
}

impl HeadingStyler for Theme {
    fn h1(&self, text: &str) -> String {
        self.heading1.apply(text).to_string()
    }

    fn h2(&self, text: &str) -> String {
        self.heading2.apply(text).to_string()
    }

    fn h3(&self, text: &str) -> String {
        self.heading3.apply(text).to_string()
    }

    fn h4(&self, text: &str) -> String {
        self.heading4.apply(text).to_string()
    }

    fn h5(&self, text: &str) -> String {
        self.heading5.apply(text).to_string()
    }

    fn h6(&self, text: &str) -> String {
        self.heading6.apply(text).to_string()
    }
}

impl ListStyler for Theme {
    fn bullet_dash(&self, text: &str) -> String {
        self.bullet_dash.apply(text).to_string()
    }

    fn bullet_asterisk(&self, text: &str) -> String {
        self.bullet_asterisk.apply(text).to_string()
    }

    fn bullet_plus(&self, text: &str) -> String {
        self.bullet_plus.apply(text).to_string()
    }

    fn bullet_plus_expand(&self, text: &str) -> String {
        self.bullet_plus_expand.apply(text).to_string()
    }

    fn number(&self, text: &str) -> String {
        self.list_number.apply(text).to_string()
    }

    fn checkbox_checked(&self, text: &str) -> String {
        self.checkbox_checked.apply(text).to_string()
    }

    fn checkbox_unchecked(&self, text: &str) -> String {
        self.checkbox_unchecked.apply(text).to_string()
    }
}

impl TableStyler for Theme {
    fn border(&self, text: &str) -> String {
        self.table_border.apply(text).to_string()
    }

    fn header(&self, text: &str) -> String {
        self.table_header.apply(text).to_string()
    }
}

impl Theme {
    /// Detects the terminal theme (dark or light) and returns the appropriate
    /// theme.
    pub fn detect() -> Self {
        use crate::utils::{ThemeMode, detect_theme_mode};

        match detect_theme_mode() {
            ThemeMode::Light => Self::light(),
            ThemeMode::Dark => Self::dark(),
        }
    }

    /// Dark theme (default).
    pub fn dark() -> Self {
        Self {
            // Inline
            bold: Style::new().bold(),
            italic: Style::new().italic(),
            code: Style::new().fg(Color::Yellow),
            strikethrough: Style::new().strikethrough().dimmed(),
            link: Style::new().fg(Color::Cyan).underline(),
            link_url: Style::new().fg(Color::Blue).dimmed(),

            // Headings
            heading1: Style::new().fg(Color::Magenta).bold(),
            heading2: Style::new().fg(Color::Blue).bold(),
            heading3: Style::new().fg(Color::Cyan).bold(),
            heading4: Style::new().fg(Color::Green).bold(),
            heading5: Style::new().fg(Color::Yellow).bold(),
            heading6: Style::new().fg(Color::White).bold(),

            // Lists
            bullet_dash: Style::new().fg(Color::Cyan),
            bullet_asterisk: Style::new().fg(Color::Green),
            bullet_plus: Style::new().fg(Color::Yellow),
            bullet_plus_expand: Style::new().fg(Color::Magenta),
            list_number: Style::new().fg(Color::Cyan),
            checkbox_checked: Style::new().fg(Color::Green),
            checkbox_unchecked: Style::new().fg(Color::Red),

            // Tables
            table_header: Style::new().bold(),
            table_border: Style::new().fg(Color::BrightBlack),
            table_cell: Style::new(),

            // Quotes
            blockquote: Style::new().italic().dimmed(),
            blockquote_border: Style::new().fg(Color::BrightBlack),
            think: Style::new().italic().fg(Color::BrightBlack),
            think_border: Style::new().fg(Color::BrightBlack),

            // Code block
            code_block_lang: Style::new().fg(Color::BrightBlack).italic(),

            // HR
            hr: Style::new().fg(Color::BrightBlack),

            // Mermaid diagrams - AA+ contrast with background colors for dark terminals
            // All mermaid styles include bg(Color::BrightBlack) so the diagram background is
            // consistent across every character.
            mermaid_node: Style::new()
                .fg(Color::BrightWhite)
                .bg(Color::BrightBlack)
                .bold(),
            mermaid_edge: Style::new().fg(Color::BrightCyan).bg(Color::BrightBlack),
            mermaid_label: Style::new()
                .fg(Color::BrightYellow)
                .bg(Color::BrightBlack)
                .italic(),
            mermaid_border: Style::new().fg(Color::BrightWhite).bg(Color::BrightBlack),
            mermaid_arrow_head: Style::new().fg(Color::BrightGreen).bg(Color::BrightBlack),
            mermaid_node_start: Style::new().fg(Color::BrightWhite).bg(Color::Green).bold(),
            mermaid_node_end: Style::new().fg(Color::BrightWhite).bg(Color::Blue).bold(),
            mermaid_node_process: Style::new().fg(Color::BrightWhite).bg(Color::Cyan).bold(),
            mermaid_node_decision: Style::new()
                .fg(Color::BrightWhite)
                .bg(Color::BrightBlack)
                .bold(),
            mermaid_edge_yes: Style::new()
                .fg(Color::BrightGreen)
                .bg(Color::BrightBlack)
                .bold(),
            mermaid_edge_no: Style::new()
                .fg(Color::BrightRed)
                .bg(Color::BrightBlack)
                .bold(),
            mermaid_bg: Style::new().bg(Color::BrightBlack),
        }
    }

    /// Light theme for light terminal backgrounds.
    pub fn light() -> Self {
        Self {
            // Inline
            bold: Style::new().bold(),
            italic: Style::new().italic(),
            code: Style::new().fg(Color::Red),
            strikethrough: Style::new().strikethrough().dimmed(),
            link: Style::new().fg(Color::Blue).underline(),
            link_url: Style::new().fg(Color::Cyan).dimmed(),

            // Headings
            heading1: Style::new().fg(Color::Magenta).bold(),
            heading2: Style::new().fg(Color::Blue).bold(),
            heading3: Style::new().fg(Color::Cyan).bold(),
            heading4: Style::new().fg(Color::Green).bold(),
            heading5: Style::new().fg(Color::Yellow).bold(),
            heading6: Style::new().fg(Color::Black).bold(),

            // Lists
            bullet_dash: Style::new().fg(Color::Blue),
            bullet_asterisk: Style::new().fg(Color::Green),
            bullet_plus: Style::new().fg(Color::Magenta),
            bullet_plus_expand: Style::new().fg(Color::Cyan),
            list_number: Style::new().fg(Color::Blue),
            checkbox_checked: Style::new().fg(Color::Green),
            checkbox_unchecked: Style::new().fg(Color::Red),

            // Tables
            table_header: Style::new().bold(),
            table_border: Style::new().fg(Color::Black),
            table_cell: Style::new(),

            // Quotes
            blockquote: Style::new().italic().dimmed(),
            blockquote_border: Style::new().fg(Color::Black),
            think: Style::new().italic().fg(Color::Black),
            think_border: Style::new().fg(Color::Black),

            // Code block
            code_block_lang: Style::new().fg(Color::Black).italic(),

            // HR
            hr: Style::new().fg(Color::Black),

            // Mermaid diagrams - AA+ contrast with background colors for light terminals
            // All mermaid styles include bg(Color::BrightWhite) so the diagram background is
            // consistent across every character.
            mermaid_node: Style::new().fg(Color::Black).bg(Color::BrightWhite).bold(),
            mermaid_edge: Style::new().fg(Color::Blue).bg(Color::BrightWhite),
            mermaid_label: Style::new()
                .fg(Color::Yellow)
                .bg(Color::BrightWhite)
                .italic(),
            mermaid_border: Style::new().fg(Color::Black).bg(Color::BrightWhite),
            mermaid_arrow_head: Style::new().fg(Color::Green).bg(Color::BrightWhite),
            mermaid_node_start: Style::new().fg(Color::Black).bg(Color::Green).bold(),
            mermaid_node_end: Style::new().fg(Color::White).bg(Color::Blue).bold(),
            mermaid_node_process: Style::new().fg(Color::Black).bg(Color::Cyan).bold(),
            mermaid_node_decision: Style::new().fg(Color::Black).bg(Color::BrightWhite).bold(),
            mermaid_edge_yes: Style::new().fg(Color::Green).bg(Color::BrightWhite).bold(),
            mermaid_edge_no: Style::new().fg(Color::Red).bg(Color::BrightWhite).bold(),
            mermaid_bg: Style::new().bg(Color::BrightWhite),
        }
    }
}

/// Test styler that outputs readable HTML-like tags.
#[cfg(test)]
pub struct TagStyler;

#[cfg(test)]
impl InlineStyler for TagStyler {
    fn text(&self, text: &str) -> String {
        decode_html_entities(text)
    }

    fn bold(&self, text: &str) -> String {
        format!("<b>{}</b>", decode_html_entities(text))
    }

    fn italic(&self, text: &str) -> String {
        format!("<i>{}</i>", decode_html_entities(text))
    }

    fn bold_italic(&self, text: &str) -> String {
        format!("<b><i>{}</i></b>", decode_html_entities(text))
    }

    fn strikethrough(&self, text: &str) -> String {
        format!("<s>{}</s>", decode_html_entities(text))
    }

    fn underline(&self, text: &str) -> String {
        format!("<u>{}</u>", decode_html_entities(text))
    }

    fn code(&self, text: &str) -> String {
        format!("<code>{}</code>", text)
    }

    fn link(&self, text: &str, url: &str) -> String {
        format!("<a href=\"{}\">{}</a>", url, decode_html_entities(text))
    }

    fn image(&self, alt: &str, url: &str) -> String {
        format!("<img alt=\"{}\" src=\"{}\"/>", alt, url)
    }

    fn footnote(&self, text: &str) -> String {
        format!("<footnote>{}</footnote>", text)
    }

    fn dimmed(&self, text: &str) -> String {
        format!("<dim>{}</dim>", text)
    }
}

#[cfg(test)]
impl HeadingStyler for TagStyler {
    fn h1(&self, text: &str) -> String {
        format!("<h1>{}</h1>", text)
    }

    fn h2(&self, text: &str) -> String {
        format!("<h2>{}</h2>", text)
    }

    fn h3(&self, text: &str) -> String {
        format!("<h3>{}</h3>", text)
    }

    fn h4(&self, text: &str) -> String {
        format!("<h4>{}</h4>", text)
    }

    fn h5(&self, text: &str) -> String {
        format!("<h5>{}</h5>", text)
    }

    fn h6(&self, text: &str) -> String {
        format!("<h6>{}</h6>", text)
    }
}

#[cfg(test)]
impl ListStyler for TagStyler {
    fn bullet_dash(&self, text: &str) -> String {
        format!("<dash>{}</dash>", text)
    }

    fn bullet_asterisk(&self, text: &str) -> String {
        format!("<asterisk>{}</asterisk>", text)
    }

    fn bullet_plus(&self, text: &str) -> String {
        format!("<plus>{}</plus>", text)
    }

    fn bullet_plus_expand(&self, text: &str) -> String {
        format!("<expand>{}</expand>", text)
    }

    fn number(&self, text: &str) -> String {
        format!("<num>{}</num>", text)
    }

    fn checkbox_checked(&self, text: &str) -> String {
        format!("<checked>{}</checked>", text)
    }

    fn checkbox_unchecked(&self, text: &str) -> String {
        format!("<unchecked>{}</unchecked>", text)
    }
}

#[cfg(test)]
impl TableStyler for TagStyler {
    fn border(&self, text: &str) -> String {
        Theme::default().border(text)
    }

    fn header(&self, text: &str) -> String {
        Theme::default().header(text)
    }
}
