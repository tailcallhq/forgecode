//! Utility functions for the markdown renderer.

use std::sync::OnceLock;
use std::time::Duration;

use streamdown_ansi::utils::{extract_ansi_codes, parse_sgr_params, visible, visible_length};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Terminal theme mode (dark or light).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    /// Dark terminal background.
    Dark,
    /// Light terminal background.
    Light,
}

/// Maximum time to wait for a terminal color query response.
const THEME_DETECT_TIMEOUT: Duration = Duration::from_millis(100);

/// Process-wide cache for the detected terminal theme mode.
static THEME_MODE: OnceLock<ThemeMode> = OnceLock::new();

/// Detects the terminal theme mode (dark or light), querying the terminal at
/// most once per process lifetime. Subsequent calls return the cached result.
/// Falls back to dark mode if the terminal does not respond within the timeout.
pub fn detect_theme_mode() -> ThemeMode {
    *THEME_MODE.get_or_init(|| {
        use terminal_colorsaurus::{QueryOptions, ThemeMode as ColorsaurusThemeMode, theme_mode};

        let mut opts = QueryOptions::default();
        opts.timeout = THEME_DETECT_TIMEOUT;
        match theme_mode(opts) {
            Ok(ColorsaurusThemeMode::Light) => ThemeMode::Light,
            Ok(ColorsaurusThemeMode::Dark) | Err(_) => ThemeMode::Dark,
        }
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WrapAtom<'a> {
    Escape(&'a str),
    Grapheme(&'a str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WrapSegment {
    separator: String,
    word: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WrapLayout<'a> {
    next_width: usize,
    first_prefix: &'a str,
    next_prefix: &'a str,
}

/// Wraps ANSI-styled text while preserving explicit whitespace between words.
///
/// Unlike the upstream streamdown wrapper, this keeps the original separator
/// string between tokens instead of reconstructing it from CJK heuristics.
pub(crate) fn wrap_text_preserving_spaces(
    text: &str,
    first_width: usize,
    next_width: usize,
    first_prefix: &str,
    next_prefix: &str,
) -> Vec<String> {
    if first_width == 0 && next_width == 0 {
        return Vec::new();
    }

    let segments = wrap_segments(text);
    if segments.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut current_line = String::new();
    let mut current_style: Vec<String> = Vec::new();
    let mut current_width = first_width;
    let layout = WrapLayout { next_width, first_prefix, next_prefix };

    for segment in segments {
        let line_width = visible_length(&current_line);
        let separator = if current_line.is_empty() {
            ""
        } else {
            segment.separator.as_str()
        };
        let combined_width = visible_length(separator) + visible_length(&segment.word);

        if !current_line.is_empty() && line_width + combined_width <= current_width {
            current_line.push_str(separator);
            apply_style_transition(&mut current_style, separator);
            current_line.push_str(&segment.word);
            apply_style_transition(&mut current_style, &segment.word);
            continue;
        }

        if current_line.is_empty() && visible_length(&segment.word) <= current_width {
            current_line.push_str(&segment.word);
            apply_style_transition(&mut current_style, &segment.word);
            continue;
        }

        if !current_line.is_empty() {
            push_wrapped_line(&mut lines, &current_line, first_prefix, next_prefix);
            current_line = current_style.join("");
            current_width = next_width;
        }

        append_wrapped_word(
            &mut lines,
            &mut current_line,
            &mut current_style,
            &segment.word,
            &mut current_width,
            layout,
        );
    }

    push_wrapped_line(&mut lines, &current_line, first_prefix, next_prefix);
    lines
}

/// Wraps ANSI-styled inline text without prefixes while preserving explicit
/// spaces.
pub(crate) fn simple_wrap_preserving_spaces(text: &str, width: usize) -> Vec<String> {
    if width == 0 || text.is_empty() {
        return vec![text.to_string()];
    }

    let lines = wrap_text_preserving_spaces(text, width, width, "", "");
    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

fn append_wrapped_word(
    lines: &mut Vec<String>,
    current_line: &mut String,
    current_style: &mut Vec<String>,
    word: &str,
    current_width: &mut usize,
    layout: WrapLayout<'_>,
) {
    let mut remainder = word;

    while !remainder.is_empty() {
        let line_width = visible_length(current_line);
        let mut available = current_width.saturating_sub(line_width);

        if available == 0 {
            push_wrapped_line(lines, current_line, layout.first_prefix, layout.next_prefix);
            *current_line = current_style.join("");
            *current_width = layout.next_width;
            available = (*current_width).max(1);
        }

        if visible_length(remainder) <= available {
            current_line.push_str(remainder);
            apply_style_transition(current_style, remainder);
            break;
        }

        let prefix = take_prefix_fitting(remainder, available)
            .or_else(|| take_prefix_fitting(remainder, 1))
            .unwrap_or(remainder);

        current_line.push_str(prefix);
        apply_style_transition(current_style, prefix);
        remainder = remainder.strip_prefix(prefix).unwrap_or_default();

        if !remainder.is_empty() {
            push_wrapped_line(lines, current_line, layout.first_prefix, layout.next_prefix);
            *current_line = current_style.join("");
            *current_width = layout.next_width;
        }
    }
}

fn take_prefix_fitting(text: &str, max_width: usize) -> Option<&str> {
    if text.is_empty() {
        return None;
    }

    let mut width = 0;
    let mut prefix_end = 0;
    let mut consumed_visible = false;

    for atom in parse_atoms(text) {
        match atom {
            WrapAtom::Escape(sequence) => prefix_end += sequence.len(),
            WrapAtom::Grapheme(grapheme) => {
                let grapheme_width = UnicodeWidthStr::width(grapheme);
                if consumed_visible && width + grapheme_width > max_width {
                    break;
                }
                if !consumed_visible && grapheme_width > max_width {
                    prefix_end += grapheme.len();
                    break;
                }

                prefix_end += grapheme.len();
                width += grapheme_width;
                consumed_visible = true;
            }
        }
    }

    (prefix_end > 0).then(|| text.get(..prefix_end)).flatten()
}

fn wrap_segments(text: &str) -> Vec<WrapSegment> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut current_is_whitespace = None;
    let mut separator = String::new();

    for atom in parse_atoms(text) {
        match atom {
            WrapAtom::Escape(sequence) => current.push_str(sequence),
            WrapAtom::Grapheme(grapheme) => {
                let is_whitespace = grapheme.chars().all(char::is_whitespace);
                match current_is_whitespace {
                    Some(kind) if kind != is_whitespace => {
                        if kind {
                            separator.push_str(&current);
                            current.clear();
                        } else {
                            segments.push(WrapSegment {
                                separator: std::mem::take(&mut separator),
                                word: std::mem::take(&mut current),
                            });
                        }
                        current_is_whitespace = Some(is_whitespace);
                    }
                    None => current_is_whitespace = Some(is_whitespace),
                    _ => {}
                }

                current.push_str(grapheme);
            }
        }
    }

    match (current_is_whitespace, current.is_empty()) {
        (Some(true), false) => separator.push_str(&current),
        (Some(false), false) => segments.push(WrapSegment { separator, word: current }),
        _ => {}
    }

    segments
}

fn parse_atoms(text: &str) -> Vec<WrapAtom<'_>> {
    let mut atoms = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        let Some(current_byte) = bytes.get(index) else {
            break;
        };

        if *current_byte != 0x1b {
            let next_escape = text
                .get(index..)
                .and_then(|s| s.find('\x1b'))
                .map(|offset| index + offset)
                .unwrap_or(text.len());

            if let Some(slice) = text.get(index..next_escape) {
                for grapheme in slice.graphemes(true) {
                    atoms.push(WrapAtom::Grapheme(grapheme));
                }
            }

            index = next_escape;
            continue;
        }

        let end = match bytes.get(index + 1) {
            Some(b'[') => parse_csi_escape(bytes, index),
            Some(b']') => parse_osc_escape(bytes, index),
            Some(_) => (index + 2).min(bytes.len()),
            None => bytes.len(),
        };

        if let Some(sequence) = text.get(index..end) {
            atoms.push(WrapAtom::Escape(sequence));
        }

        index = end;
    }

    atoms
}

fn parse_csi_escape(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 2;
    while let Some(byte) = bytes.get(index) {
        if (0x40..=0x7e).contains(byte) {
            return index + 1;
        }

        index += 1;
    }

    bytes.len()
}

fn parse_osc_escape(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 2;
    while let Some(byte) = bytes.get(index) {
        if *byte == 0x07 {
            return index + 1;
        }

        if *byte == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
            return index + 2;
        }

        index += 1;
    }

    bytes.len()
}

fn apply_style_transition(current_style: &mut Vec<String>, text: &str) {
    current_style.extend(extract_ansi_codes(text));
    *current_style = collapse_ansi_codes(current_style);
}

fn collapse_ansi_codes(code_list: &[String]) -> Vec<String> {
    let mut bold = false;
    let mut italic = false;
    let mut underline = false;
    let mut strikeout = false;
    let mut dim = false;
    let mut fg_color: Option<String> = None;
    let mut bg_color: Option<String> = None;

    for code in code_list {
        let params = parse_sgr_params(code);
        let mut index = 0;

        while let Some(&param) = params.get(index) {
            match param {
                0 => {
                    bold = false;
                    italic = false;
                    underline = false;
                    strikeout = false;
                    dim = false;
                    fg_color = None;
                    bg_color = None;
                }
                1 => bold = true,
                2 => dim = true,
                3 => italic = true,
                4 => underline = true,
                9 => strikeout = true,
                22 => {
                    bold = false;
                    dim = false;
                }
                23 => italic = false,
                24 => underline = false,
                29 => strikeout = false,
                30..=37 | 90..=97 => fg_color = Some(format!("\x1b[{param}m")),
                39 => fg_color = None,
                40..=47 | 100..=107 => bg_color = Some(format!("\x1b[{param}m")),
                49 => bg_color = None,
                38 => {
                    if let Some([2, red, green, blue]) = params.get(index + 1..index + 5) {
                        fg_color = Some(format!("\x1b[38;2;{red};{green};{blue}m"));
                        index += 4;
                    }
                }
                48 => {
                    if let Some([2, red, green, blue]) = params.get(index + 1..index + 5) {
                        bg_color = Some(format!("\x1b[48;2;{red};{green};{blue}m"));
                        index += 4;
                    }
                }
                _ => {}
            }

            index += 1;
        }
    }

    let mut result = Vec::new();
    let mut sgr_parts = Vec::new();

    if bold {
        sgr_parts.push("1");
    }
    if dim {
        sgr_parts.push("2");
    }
    if italic {
        sgr_parts.push("3");
    }
    if underline {
        sgr_parts.push("4");
    }
    if strikeout {
        sgr_parts.push("9");
    }

    if !sgr_parts.is_empty() {
        result.push(format!("\x1b[{}m", sgr_parts.join(";")));
    }
    if let Some(fg_color) = fg_color {
        result.push(fg_color);
    }
    if let Some(bg_color) = bg_color {
        result.push(bg_color);
    }

    result
}

fn push_wrapped_line(
    lines: &mut Vec<String>,
    current_line: &str,
    first_prefix: &str,
    next_prefix: &str,
) {
    if visible(current_line).trim().is_empty() {
        return;
    }

    let prefix = if lines.is_empty() {
        first_prefix
    } else {
        next_prefix
    };
    lines.push(format!("{prefix}{current_line}"));
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use streamdown_ansi::utils::visible;

    use super::{simple_wrap_preserving_spaces, wrap_text_preserving_spaces};

    #[test]
    fn test_simple_wrap_preserving_spaces_keeps_korean_word_boundaries() {
        let fixture = "한글 공백 보존 문장";
        let actual = simple_wrap_preserving_spaces(fixture, 8);
        let expected = vec![
            "한글".to_string(),
            "공백".to_string(),
            "보존".to_string(),
            "문장".to_string(),
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_simple_wrap_preserving_spaces_splits_long_tokens() {
        let fixture = "supercalifragilistic";
        let actual = simple_wrap_preserving_spaces(fixture, 5);
        let expected = vec![
            "super".to_string(),
            "calif".to_string(),
            "ragil".to_string(),
            "istic".to_string(),
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_text_preserving_spaces_keeps_multiple_spaces_on_same_line() {
        let fixture = "한글  공백 보존";
        let actual = wrap_text_preserving_spaces(fixture, 40, 40, "", "");
        let expected = vec!["한글  공백 보존".to_string()];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_text_preserving_spaces_applies_prefixes_after_wrap() {
        let fixture = "한글 공백 검증";
        let actual = wrap_text_preserving_spaces(fixture, 4, 4, "> ", "  ");
        let expected = vec![
            "> 한글".to_string(),
            "  공백".to_string(),
            "  검증".to_string(),
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_text_preserving_spaces_preserves_link_separator_after_osc_escape() {
        let fixture = concat!(
            "\x1b]8;;https://example.com\x1b\\",
            "link",
            "\x1b]8;;\x1b\\",
            " ",
            "\x1b[34m(https://x.co)\x1b[39m"
        );
        let actual = wrap_text_preserving_spaces(fixture, 4, 14, "", "")
            .into_iter()
            .map(|line| visible(&line))
            .collect::<Vec<_>>();
        let expected = vec!["link".to_string(), "(https://x.co)".to_string()];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_simple_wrap_preserving_spaces_keeps_grapheme_clusters_intact() {
        let fixture = "👨‍👩‍👧‍👦 a\u{0301} 한글";
        let actual = simple_wrap_preserving_spaces(fixture, 2);
        let expected = vec![
            "👨‍👩‍👧‍👦".to_string(),
            "a\u{0301}".to_string(),
            "한".to_string(),
            "글".to_string(),
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_text_preserving_spaces_reapplies_ansi_style_after_wrap() {
        let fixture = "\x1b[31mabcdef\x1b[39m";
        let actual = wrap_text_preserving_spaces(fixture, 3, 3, "", "");
        let expected_visible = vec!["abc".to_string(), "def".to_string()];
        let actual_visible = actual.iter().map(|line| visible(line)).collect::<Vec<_>>();

        assert_eq!(actual_visible, expected_visible);
        assert!(actual[0].contains("\x1b[31m"));
        assert!(actual[1].contains("\x1b[31m"));
        assert!(actual[1].ends_with("\x1b[39m"));
    }
}
