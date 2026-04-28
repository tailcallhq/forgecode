//! Main renderer that handles all parse events.

use std::io::{self, Write};

use streamdown_ansi::utils::visible_length;
use streamdown_parser::ParseEvent;

use crate::code::CodeHighlighter;
use crate::heading::render_heading;
use crate::inline::{render_inline_content, render_inline_elements};
use crate::list::{ListState, render_list_item};
use crate::mermaid::render_mermaid;
use crate::style::InlineStyler;
use crate::table::render_table;
use crate::theme::Theme;
use crate::utils::wrap_text_preserving_spaces;

/// Main renderer for markdown events.
pub struct Renderer<W: Write> {
    writer: W,
    width: usize,
    theme: Theme,
    // Code highlighting
    highlighter: CodeHighlighter,
    current_language: Option<String>,
    code_buffer: String,
    // Mermaid diagram state
    in_mermaid: bool,
    mermaid_buffer: String,
    // Table buffering
    table_rows: Vec<Vec<String>>,
    // Blockquote state
    in_blockquote: bool,
    blockquote_depth: usize,
    // List state
    list_state: ListState,
    // Column tracking
    column: usize,
}

impl<W: Write> Renderer<W> {
    pub fn new(writer: W, width: usize) -> Self {
        Self::with_theme(writer, width, Theme::default())
    }

    pub fn with_theme(writer: W, width: usize, theme: Theme) -> Self {
        Self {
            writer,
            width,
            theme,
            highlighter: CodeHighlighter::default(),
            current_language: None,
            code_buffer: String::new(),
            in_mermaid: false,
            mermaid_buffer: String::new(),
            table_rows: Vec::new(),
            in_blockquote: false,
            blockquote_depth: 0,
            list_state: ListState::default(),
            column: 0,
        }
    }

    /// Set a new theme.
    #[allow(dead_code)]
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    /// Get the current theme.
    #[allow(dead_code)]
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Calculate the left margin based on blockquote depth.
    fn left_margin(&self) -> String {
        if self.in_blockquote {
            let border = self.theme.blockquote_border.apply("│").to_string();
            format!("{} ", border).repeat(self.blockquote_depth)
        } else {
            String::new()
        }
    }

    /// Calculate the current available width.
    fn current_width(&self) -> usize {
        let margin_width = if self.in_blockquote {
            self.blockquote_depth * 3
        } else {
            0
        };
        self.width.saturating_sub(margin_width)
    }

    fn write(&mut self, s: &str) -> io::Result<()> {
        write!(self.writer, "{}", s)
    }

    fn writeln(&mut self, s: &str) -> io::Result<()> {
        writeln!(self.writer, "{}", s)?;
        self.column = 0;
        Ok(())
    }

    fn flush_table(&mut self) -> io::Result<()> {
        if self.table_rows.is_empty() {
            return Ok(());
        }
        let rows = std::mem::take(&mut self.table_rows);
        let margin = self.left_margin();
        let lines = render_table(&rows, &margin, &self.theme, self.width);
        for line in lines {
            self.writeln(&line)?;
        }
        Ok(())
    }

    /// Check if this event should reset a pending list.
    /// List continues only for ListItem, ListEnd, and EmptyLine/Newline events.
    fn should_reset_list(event: &ParseEvent) -> bool {
        !matches!(
            event,
            ParseEvent::ListItem { .. }
                | ParseEvent::ListEnd
                | ParseEvent::EmptyLine
                | ParseEvent::Newline
        )
    }

    /// Render a single parse event.
    pub fn render_event(&mut self, event: &ParseEvent) -> io::Result<()> {
        // Reset pending list if this event breaks the list context
        if Self::should_reset_list(event) {
            self.list_state.reset();
        }

        match event {
            // === Inline elements ===
            ParseEvent::Text(text) => {
                let styled = self.theme.text(text);
                self.write(&styled)?;
                self.column += styled.chars().count();
            }

            ParseEvent::InlineCode(code) => {
                self.write(&self.theme.code(code))?;
            }

            ParseEvent::Bold(text) => {
                self.write(&self.theme.bold(text))?;
            }

            ParseEvent::Italic(text) => {
                self.write(&self.theme.italic(text))?;
            }

            ParseEvent::BoldItalic(text) => {
                self.write(&self.theme.bold_italic(text))?;
            }

            ParseEvent::Underline(text) => {
                self.write(&self.theme.underline(text))?;
            }

            ParseEvent::Strikeout(text) => {
                self.write(&self.theme.strikethrough(text))?;
            }

            ParseEvent::Link { text, url } => {
                self.write(&self.theme.link(text, url))?;
            }

            ParseEvent::Image { alt, url } => {
                self.write(&self.theme.image(alt, url))?;
            }

            ParseEvent::Footnote(superscript) => {
                self.write(&self.theme.footnote(superscript))?;
            }

            ParseEvent::Prompt(prompt) => {
                self.write(prompt)?;
            }

            // === Block elements ===
            ParseEvent::Heading { level, content } => {
                let margin = self.left_margin();
                let width = self.current_width();
                let lines = render_heading(*level, content, width, &margin, &self.theme);
                for line in lines {
                    self.writeln(&line)?;
                }
            }

            ParseEvent::CodeBlockStart { language, .. } => {
                self.current_language = language.clone();
                self.code_buffer.clear();
                // Detect mermaid diagram blocks
                self.in_mermaid = language.as_deref() == Some("mermaid")
                    || language.as_deref() == Some("mermaid");
                self.mermaid_buffer.clear();
                // Show language label for mermaid blocks
                if self.in_mermaid {
                    let lang_label = self
                        .theme
                        .code_block_lang
                        .apply(&format!(" {} ", language.as_deref().unwrap_or("mermaid")));
                    self.writeln(&lang_label.to_string())?;
                }
            }

            ParseEvent::CodeBlockLine(line) => {
                if self.in_mermaid {
                    // Buffer mermaid lines instead of rendering inline
                    if !self.mermaid_buffer.is_empty() {
                        self.mermaid_buffer.push('\n');
                    }
                    self.mermaid_buffer.push_str(line);
                } else {
                    if !self.code_buffer.is_empty() {
                        self.code_buffer.push('\n');
                    }
                    self.code_buffer.push_str(line);

                    let margin = self.left_margin();
                    let width = self.current_width();
                    let rendered_lines = self.highlighter.render_code_line(
                        line,
                        self.current_language.as_deref(),
                        &margin,
                        width,
                    );
                    for rendered in rendered_lines {
                        self.writeln(&rendered)?;
                    }
                }
            }

            ParseEvent::CodeBlockEnd => {
                if self.in_mermaid {
                    // Render the complete mermaid diagram
                    if !self.mermaid_buffer.is_empty() {
                        if let Some(diagram_lines) =
                            render_mermaid(&self.mermaid_buffer, self.current_width())
                        {
                            for line in diagram_lines {
                                let margin = self.left_margin();
                                self.writeln(&format!("{}{}", margin, line))?;
                            }
                        } else {
                            // Fallback: render as code if mermaid parsing fails
                            let margin = self.left_margin();
                            let width = self.current_width();
                            let lines: Vec<String> =
                                self.mermaid_buffer.lines().map(|l| l.to_string()).collect();
                            for mermaid_line in lines {
                                let rendered_lines = self.highlighter.render_code_line(
                                    &mermaid_line,
                                    Some("text"),
                                    &margin,
                                    width,
                                );
                                for rendered in rendered_lines {
                                    self.writeln(&rendered)?;
                                }
                            }
                        }
                    }
                    self.in_mermaid = false;
                    self.mermaid_buffer.clear();
                }
                self.current_language = None;
                self.code_buffer.clear();
            }

            ParseEvent::ListItem { indent, bullet, content } => {
                let margin = self.left_margin();
                let width = self.current_width();
                let lines = render_list_item(
                    *indent,
                    bullet,
                    content,
                    width,
                    &margin,
                    &self.theme,
                    &mut self.list_state,
                );
                for line in lines {
                    self.writeln(&line)?;
                }
            }

            ParseEvent::ListEnd => {
                // Mark as pending - will reset if non-list event arrives
                self.list_state.mark_pending_reset();
            }

            ParseEvent::TableHeader(cols) | ParseEvent::TableRow(cols) => {
                self.table_rows.push(cols.clone());
            }

            ParseEvent::TableSeparator => {}

            ParseEvent::TableEnd => {
                self.flush_table()?;
            }

            ParseEvent::BlockquoteStart { depth } => {
                self.in_blockquote = true;
                self.blockquote_depth = *depth;
            }

            ParseEvent::BlockquoteLine(text) => {
                let margin = self.left_margin();
                let content_width = self.width.saturating_sub(visible_length(&margin));
                // Parse inline formatting (bold, italic, etc.) in blockquote content
                let rendered_content = render_inline_content(text, &self.theme);
                let wrapped = wrap_text_preserving_spaces(
                    &rendered_content,
                    content_width,
                    content_width,
                    &margin,
                    &margin,
                );
                if wrapped.is_empty() {
                    self.writeln(&margin)?;
                } else {
                    for line in wrapped {
                        self.writeln(&line)?;
                    }
                }
            }

            ParseEvent::BlockquoteEnd => {
                self.in_blockquote = false;
                self.blockquote_depth = 0;
            }

            ParseEvent::ThinkBlockStart => {
                self.writeln(&self.theme.think_border.apply("┌─ thinking ─").to_string())?;
                self.in_blockquote = true;
                self.blockquote_depth = 1;
            }

            ParseEvent::ThinkBlockLine(text) => {
                let border = self.theme.think_border.apply("│").to_string();
                self.writeln(&format!("{} {}", border, self.theme.think.apply(text)))?;
            }

            ParseEvent::ThinkBlockEnd => {
                self.writeln(&self.theme.think_border.apply("└").to_string())?;
                self.in_blockquote = false;
                self.blockquote_depth = 0;
            }

            ParseEvent::HorizontalRule => {
                let margin = self.left_margin();
                let rule = "─".repeat(self.current_width());
                self.writeln(&format!("{}{}", margin, self.theme.hr.apply(&rule)))?;
            }

            ParseEvent::EmptyLine | ParseEvent::Newline => {
                self.writeln("")?;
            }
            ParseEvent::InlineElements(elements) => {
                self.write(&render_inline_elements(elements, &self.theme))?;
            }
        }

        self.writer.flush()
    }
}
