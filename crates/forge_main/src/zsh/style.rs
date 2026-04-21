//! ZSH prompt styling utilities.
//!
//! This module provides helpers for generating ZSH-native prompt escape
//! sequences. Unlike ANSI escape codes, ZSH prompt escapes are interpreted by
//! ZSH's prompt renderer, making them work correctly in PROMPT and RPROMPT
//! contexts.

use std::fmt::{self, Display};

/// ZSH prompt color using 256-color palette.
///
/// Maps to ZSH's `%F{N}` prompt escape sequence where N is a color code.
#[derive(Debug, Clone, Copy)]
pub struct ZshColor(u8);

impl ZshColor {
    /// White (color 15)
    pub const WHITE: Self = Self(15);
    /// Cyan (color 134)
    pub const CYAN: Self = Self(134);
    /// Green (color 2)
    pub const GREEN: Self = Self(2);
    /// Yellow (color 3)
    pub const YELLOW: Self = Self(3);
    /// Dimmed gray (color 240)
    pub const DIMMED: Self = Self(240);

    /// Creates a color from a 256-color palette value.
    #[cfg(test)]
    pub const fn new(value: u8) -> Self {
        Self(value)
    }
}

impl Display for ZshColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A styled string for ZSH prompts.
///
/// Wraps text with ZSH prompt escape sequences for colors and formatting.
#[derive(Debug, Clone)]
pub struct ZshStyled<'a> {
    text: &'a str,
    fg: Option<ZshColor>,
    bold: bool,
}

impl<'a> ZshStyled<'a> {
    /// Creates a new styled string with the given text.
    pub fn new(text: &'a str) -> Self {
        Self { text, fg: None, bold: false }
    }

    /// Sets the foreground color.
    pub fn fg(mut self, color: ZshColor) -> Self {
        self.fg = Some(color);
        self
    }

    /// Makes the text bold.
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }
}

impl Display for ZshStyled<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Opening escapes
        if self.bold {
            write!(f, "%B")?;
        }
        if let Some(ref color) = self.fg {
            write!(f, "%F{{{}}}", color)?;
        }

        // Text content
        write!(f, "{}", self.text)?;

        // Closing escapes (in reverse order)
        if self.fg.is_some() {
            write!(f, "%f")?;
        }
        if self.bold {
            write!(f, "%b")?;
        }

        Ok(())
    }
}

/// Extension trait for styling strings for ZSH prompts.
pub trait ZshStyle {
    /// Creates a ZSH-styled wrapper for this string.
    fn zsh(&self) -> ZshStyled<'_>;
}

impl ZshStyle for str {
    fn zsh(&self) -> ZshStyled<'_> {
        ZshStyled::new(self)
    }
}

impl ZshStyle for String {
    fn zsh(&self) -> ZshStyled<'_> {
        ZshStyled::new(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text() {
        let actual = "hello".zsh().to_string();
        let expected = "hello";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_bold() {
        let actual = "hello".zsh().bold().to_string();
        let expected = "%Bhello%b";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_bold_and_color() {
        let actual = "hello".zsh().bold().fg(ZshColor::WHITE).to_string();
        let expected = "%B%F{15}hello%f%b";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_fixed_color() {
        let actual = "hello".zsh().fg(ZshColor::new(240)).to_string();
        let expected = "%F{240}hello%f";
        assert_eq!(actual, expected);
    }
}
