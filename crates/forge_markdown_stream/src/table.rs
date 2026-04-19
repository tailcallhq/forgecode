//! Table rendering with box-drawing characters.

use streamdown_ansi::utils::visible_length;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::inline::render_inline_content;
use crate::style::{InlineStyler, TableStyler};

/// Render a table with proper column widths, shrinking and wrapping if needed.
pub fn render_table<S: TableStyler + InlineStyler>(
    rows: &[Vec<String>],
    margin: &str,
    styler: &S,
    max_width: usize,
) -> Vec<String> {
    // First, render all cells with inline markdown
    let rendered_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| render_inline_content(cell, styler))
                .collect()
        })
        .collect();
    let n = rendered_rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if rendered_rows.is_empty() || n == 0 {
        return vec![];
    }

    // Calculate column widths based on rendered content
    let mut w: Vec<usize> = vec![0; n];
    for row in &rendered_rows {
        for (i, cell) in row.iter().enumerate() {
            let new_width = visible_length(cell);
            if let Some(wi) = w.get_mut(i) {
                *wi = (*wi).max(new_width);
            }
        }
    }

    // Shrink columns if table exceeds max width
    const MIN_COL_WIDTH: usize = 5;
    let overhead = margin.width() + 1 + 3 * n;
    let total: usize = w.iter().sum();
    if overhead + total > max_width && max_width > overhead {
        let avail = max_width - overhead;
        // Cap at natural width so narrow columns (e.g. "#") aren't inflated
        // up to MIN_COL_WIDTH when the proportional share rounds below it.
        w.iter_mut()
            .for_each(|x| *x = (*x * avail / total).max(MIN_COL_WIDTH).min(*x));

        // Clamping to MIN_COL_WIDTH can push the new total over `avail` when
        // tiny columns get bumped up. Trim 1 char at a time from the widest
        // column (above the minimum) until it fits.
        let mut excess = w.iter().sum::<usize>().saturating_sub(avail);
        while excess > 0 {
            let Some(v) = w
                .iter_mut()
                .filter(|v| **v > MIN_COL_WIDTH)
                .max_by_key(|v| **v)
            else {
                break;
            };
            *v -= 1;
            excess -= 1;
        }
    }

    // Helper to create horizontal lines
    let hline = |l: &str, m: &str, r: &str| {
        format!(
            "{}{}{}{}",
            margin,
            styler.border(l),
            w.iter()
                .map(|&x| styler.border(&"─".repeat(x + 2)))
                .collect::<Vec<_>>()
                .join(&styler.border(m)),
            styler.border(r)
        )
    };

    let mut out = vec![hline("┌", "┬", "┐")];

    for (ri, row) in rendered_rows.iter().enumerate() {
        // Wrap each cell's content
        let wrapped: Vec<Vec<String>> = (0..n)
            .map(|i| {
                let width = w.get(i).copied().unwrap_or(0);
                wrap(row.get(i).map(|s| s.as_str()).unwrap_or(""), width)
            })
            .collect();

        // Render each line of the wrapped cells
        for li in 0..wrapped.iter().map(|c| c.len()).max().unwrap_or(1) {
            let cells: String = (0..n)
                .map(|i| {
                    let c = wrapped
                        .get(i)
                        .and_then(|w| w.get(li))
                        .map(|s| s.as_str())
                        .unwrap_or("");
                    let width = w.get(i).copied().unwrap_or(0);
                    let p = " ".repeat(width.saturating_sub(visible_length(c)));
                    if ri == 0 && li == 0 && !c.is_empty() {
                        format!(" {}{} ", styler.header(c), p)
                    } else {
                        format!(" {}{} ", c, p)
                    }
                })
                .collect::<Vec<_>>()
                .join(&styler.border("│"));
            out.push(format!(
                "{}{}{}{}",
                margin,
                styler.border("│"),
                cells,
                styler.border("│")
            ));
        }

        // Add row separator (except after last row)
        if ri < rendered_rows.len() - 1 {
            out.push(hline("├", "┼", "┤"));
        }
    }

    out.push(hline("└", "┴", "┘"));
    out
}

/// Wrap text by words, preserving ANSI codes across lines.
/// Breaks at spaces, and tries to keep content together when possible.
/// Handles both CSI sequences (\x1b[...m) and OSC sequences (\x1b]...\x1b\\).
fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 || visible_length(text) <= width {
        return vec![text.to_string()];
    }

    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0;
    let mut word = String::new();
    let mut word_width = 0;
    let mut esc = String::new();
    let mut in_osc = false;
    let mut active_style: Option<String> = None;

    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars.get(i).copied().unwrap_or('\0');

        // Handle escape sequences
        if c == '\x1b' {
            esc.push(c);
            i += 1;

            // Check what type of sequence
            if i < chars.len() {
                let next = chars.get(i).copied().unwrap_or('\0');
                esc.push(next);
                i += 1;

                if next == '[' {
                    // CSI sequence - read until 'm' or other terminator
                    while i < chars.len() {
                        let sc = chars.get(i).copied().unwrap_or('\0');
                        esc.push(sc);
                        i += 1;
                        if sc == 'm' || sc == 'K' || sc == 'H' || sc == 'J' {
                            break;
                        }
                    }
                    // Track active style for CSI color/style sequences
                    if esc.ends_with('m') {
                        active_style = if esc == "\x1b[0m" {
                            None
                        } else {
                            Some(esc.clone())
                        };
                    }
                    word.push_str(&esc);
                    esc.clear();
                } else if next == ']' {
                    // OSC sequence - read until \x1b\\
                    in_osc = true;
                    while i < chars.len() {
                        let sc = chars.get(i).copied().unwrap_or('\0');
                        esc.push(sc);
                        i += 1;
                        if sc == '\\' && esc.len() >= 2 {
                            let prev = esc.chars().rev().nth(1);
                            if prev == Some('\x1b') {
                                in_osc = false;
                                break;
                            }
                        }
                        // Also check for BEL terminator
                        if sc == '\x07' {
                            in_osc = false;
                            break;
                        }
                    }
                    word.push_str(&esc);
                    esc.clear();
                } else if next == '\\' && in_osc {
                    // End of OSC sequence
                    word.push_str(&esc);
                    esc.clear();
                    in_osc = false;
                } else {
                    // Unknown sequence, just add it
                    word.push_str(&esc);
                    esc.clear();
                }
            }
            continue;
        }

        let cw = c.width().unwrap_or(0);

        // Check if this is a word boundary (space)
        if c.is_whitespace() {
            // Gate on `line_width` (visible), not `line.is_empty()`: when an
            // active style is set `line` carries an ANSI prefix with zero
            // visible chars, and pushing it would render as a blank row.
            if line_width > 0 && line_width + word_width + cw > width {
                line.push_str("\x1b[0m");
                lines.push(line);
                line = active_style.clone().unwrap_or_default();
                line_width = 0;
            }
            line.push_str(&word);
            line_width += word_width;
            // Skip the separator if it would push past `width` — otherwise a
            // word that fills the column exactly leaves a trailing space that
            // overflows the cell by one char.
            if line_width + cw <= width {
                line.push(c);
                line_width += cw;
            }
            word.clear();
            word_width = 0;
        } else {
            // Add character to current word
            word.push(c);
            word_width += cw;

            // If word itself exceeds width, break it by character
            if word_width > width {
                if line_width > 0 {
                    line.push_str("\x1b[0m");
                    lines.push(line);
                    line = active_style.clone().unwrap_or_default();
                    line_width = 0;
                }
                // Push the long word, breaking at width while preserving ANSI codes
                while visible_length(&word) > width {
                    let (chunk, rem) = split_word_at_width(&word, width);
                    if !chunk.is_empty() {
                        line.push_str(&chunk);
                        line.push_str("\x1b[0m");
                        lines.push(line);
                        line = active_style.clone().unwrap_or_default();
                        line_width = 0;
                    }
                    word = rem;
                }
                word_width = visible_length(&word);
            }
        }
        i += 1;
    }

    // Add remaining word to line
    if !word.is_empty() {
        if line_width + word_width > width && line_width > 0 {
            line.push_str("\x1b[0m");
            lines.push(line);
            line = active_style.clone().unwrap_or_default();
            line.push_str(&word);
            line_width = word_width;
        } else {
            line.push_str(&word);
            line_width += word_width;
        }
    }

    if line_width > 0 {
        lines.push(line);
    }

    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

/// Split a word at a given visible width, preserving ANSI escape sequences.
fn split_word_at_width(word: &str, width: usize) -> (String, String) {
    let mut chunk = String::new();
    let mut chunk_w = 0;
    let mut remaining = String::new();
    let mut in_chunk = true;
    let mut esc = String::new();

    let chars: Vec<char> = word.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars.get(i).copied().unwrap_or('\0');

        // Handle escape sequences
        if c == '\x1b' {
            esc.push(c);
            i += 1;

            if i < chars.len() {
                let next = chars.get(i).copied().unwrap_or('\0');
                esc.push(next);
                i += 1;

                if next == '[' {
                    // CSI sequence
                    while i < chars.len() {
                        let sc = chars.get(i).copied().unwrap_or('\0');
                        esc.push(sc);
                        i += 1;
                        if sc == 'm' || sc == 'K' || sc == 'H' || sc == 'J' {
                            break;
                        }
                    }
                } else if next == ']' {
                    // OSC sequence - read until \x1b\\ or BEL
                    while i < chars.len() {
                        let sc = chars.get(i).copied().unwrap_or('\0');
                        esc.push(sc);
                        i += 1;
                        if sc == '\\' && esc.len() >= 2 {
                            let prev_idx = esc.len() - 2;
                            if esc.chars().nth(prev_idx) == Some('\x1b') {
                                break;
                            }
                        }
                        if sc == '\x07' {
                            break;
                        }
                    }
                }
            }

            // Add escape sequence to appropriate part
            if in_chunk {
                chunk.push_str(&esc);
            } else {
                remaining.push_str(&esc);
            }
            esc.clear();
            continue;
        }

        if in_chunk {
            let cw = c.width().unwrap_or(0);
            if chunk_w + cw <= width {
                chunk.push(c);
                chunk_w += cw;
            } else {
                remaining.push(c);
                in_chunk = false;
            }
        } else {
            remaining.push(c);
        }
        i += 1;
    }

    (chunk, remaining)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{TagStyler, Theme};

    fn strip_ansi(s: &str) -> String {
        let bytes = strip_ansi_escapes::strip(s);
        String::from_utf8(bytes).unwrap()
    }

    fn render(rows: Vec<Vec<&str>>) -> String {
        let rows: Vec<Vec<String>> = rows
            .into_iter()
            .map(|r| r.into_iter().map(|s| s.to_string()).collect())
            .collect();
        let output = render_table(&rows, "  ", &TagStyler, 80).join("\n");
        strip_ansi(&output)
    }

    fn render_with_width(rows: Vec<Vec<&str>>, width: usize) -> String {
        let rows: Vec<Vec<String>> = rows
            .into_iter()
            .map(|r| r.into_iter().map(|s| s.to_string()).collect())
            .collect();
        let output = render_table(&rows, "  ", &TagStyler, width).join("\n");
        strip_ansi(&output)
    }

    fn render_with_margin(rows: Vec<Vec<&str>>, margin: &str) -> String {
        let rows: Vec<Vec<String>> = rows
            .into_iter()
            .map(|r| r.into_iter().map(|s| s.to_string()).collect())
            .collect();
        let output = render_table(&rows, margin, &TagStyler, 80).join("\n");
        strip_ansi(&output)
    }

    #[test]
    fn test_simple_table() {
        insta::assert_snapshot!(render(vec![vec!["Name", "Age"], vec!["Alice", "30"],]));
    }

    #[test]
    fn test_single_cell() {
        insta::assert_snapshot!(render(vec![vec!["Header"], vec!["Value"],]));
    }

    #[test]
    fn test_three_columns() {
        insta::assert_snapshot!(render(vec![
            vec!["A", "B", "C"],
            vec!["1", "2", "3"],
            vec!["x", "y", "z"],
        ]));
    }

    #[test]
    fn test_varying_column_widths() {
        insta::assert_snapshot!(render(vec![
            vec!["Short", "Much Longer Header"],
            vec!["a", "b"],
        ]));
    }

    #[test]
    fn test_empty_cells() {
        insta::assert_snapshot!(render(vec![
            vec!["A", "B"],
            vec!["", "value"],
            vec!["data", ""],
        ]));
    }

    #[test]
    fn test_empty_table() {
        let rows: Vec<Vec<String>> = vec![];
        let result = render_table(&rows, "  ", &Theme::dark(), 80);
        assert!(result.is_empty());
    }

    #[test]
    fn test_empty_row() {
        let rows: Vec<Vec<String>> = vec![vec![]];
        let result = render_table(&rows, "  ", &Theme::dark(), 80);
        assert!(result.is_empty());
    }

    #[test]
    fn test_custom_margin() {
        insta::assert_snapshot!(render_with_margin(
            vec![vec!["A", "B"], vec!["1", "2"],],
            "    "
        ));
    }

    #[test]
    fn test_no_margin() {
        insta::assert_snapshot!(render_with_margin(
            vec![vec!["A", "B"], vec!["1", "2"],],
            ""
        ));
    }

    #[test]
    fn test_narrow_width_shrinks_columns() {
        insta::assert_snapshot!(render_with_width(
            vec![
                vec!["Long Header One", "Long Header Two"],
                vec!["value1", "value2"],
            ],
            40
        ));
    }

    #[test]
    fn test_narrow_first_col_not_padded() {
        insta::assert_snapshot!(render_with_width(
            vec![
                vec!["#", "File", "Description"],
                vec![
                    "1",
                    "plans/foo.md",
                    "A longer description that forces the table to shrink its columns to fit the target width"
                ],
                vec![
                    "2",
                    "docs/bar.md",
                    "Another reasonably long description so the Description column stays the widest"
                ],
            ],
            80
        ));
    }

    #[test]
    fn test_wrap_word_exactly_fills_width_drops_trailing_space() {
        // A word whose width equals `width` followed by a space must not keep
        // the space — it would push visible width to `width + 1` and overflow
        // the cell border by one char.
        let result = wrap("abcdefgh more text", 8);
        let strip = |s: &str| String::from_utf8(strip_ansi_escapes::strip(s)).unwrap();
        assert_eq!(strip(&result[0]), "abcdefgh");
        for line in &result {
            assert!(visible_length(line) <= 8);
        }
    }

    #[test]
    fn test_styled_content_no_blank_middle_line() {
        // An inherited ANSI style prefix on a fresh line has zero visible
        // chars, so it must not be pushed as its own wrapped row.
        let wrapped = wrap("\x1b[33mdocs/pdb-reference.md\x1b[0m", 8);
        let stripped: Vec<String> = wrapped
            .iter()
            .map(|s| String::from_utf8(strip_ansi_escapes::strip(s)).unwrap())
            .collect();
        assert_eq!(stripped, vec!["docs/pdb", "-referen", "ce.md"]);
    }

    #[test]
    fn test_unicode_content() {
        insta::assert_snapshot!(render(vec![vec!["名前", "年齢"], vec!["田中", "25"],]));
    }

    #[test]
    fn test_single_row_header_only() {
        insta::assert_snapshot!(render(vec![vec!["Only", "Headers"],]));
    }

    #[test]
    fn test_many_rows() {
        insta::assert_snapshot!(render(vec![
            vec!["ID", "Value"],
            vec!["1", "one"],
            vec!["2", "two"],
            vec!["3", "three"],
            vec!["4", "four"],
        ]));
    }

    // ==================== wrap function tests ====================

    #[test]
    fn test_wrap_no_wrap_needed() {
        let result = wrap("hello", 10);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn test_wrap_exact_width() {
        let result = wrap("hello", 5);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn test_wrap_splits_text() {
        let result = wrap("hello world", 5);
        assert_eq!(result.len(), 2);
        let strip = |s: &str| String::from_utf8(strip_ansi_escapes::strip(s)).unwrap();
        assert_eq!(strip(&result[0]), "hello");
        assert_eq!(strip(&result[1]), "world");
    }

    #[test]
    fn test_wrap_empty() {
        let result = wrap("", 10);
        assert_eq!(result, vec![""]);
    }

    #[test]
    fn test_wrap_zero_width() {
        let result = wrap("hello", 0);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn test_wrap_unicode() {
        // Chinese chars are 2 wide each
        let result = wrap("你好世界", 4);
        assert_eq!(result.len(), 2); // "你好" and "世界"
    }

    // ==================== Edge cases ====================

    #[test]
    fn test_long_content_in_cells() {
        insta::assert_snapshot!(render(vec![
            vec!["Header", "Description"],
            vec![
                "Short",
                "This is a much longer piece of content that should demonstrate how the table handles varying content lengths"
            ],
        ]));
    }

    #[test]
    fn test_multiline_content_wrapping() {
        insta::assert_snapshot!(render_with_width(
            vec![
                vec!["Name", "Bio"],
                vec!["Alice", "Software engineer with 10 years of experience"],
                vec!["Bob", "Data scientist specializing in machine learning"],
            ],
            50
        ));
    }

    #[test]
    fn test_very_narrow_width() {
        insta::assert_snapshot!(render_with_width(
            vec![vec!["Column A", "Column B"], vec!["value1", "value2"],],
            25
        ));
    }

    #[test]
    fn test_single_column_many_rows() {
        insta::assert_snapshot!(render(vec![
            vec!["Items"],
            vec!["First"],
            vec!["Second"],
            vec!["Third"],
            vec!["Fourth"],
            vec!["Fifth"],
        ]));
    }

    #[test]
    fn test_wide_table_many_columns() {
        insta::assert_snapshot!(render(vec![
            vec!["A", "B", "C", "D", "E", "F"],
            vec!["1", "2", "3", "4", "5", "6"],
        ]));
    }

    #[test]
    fn test_uneven_row_lengths() {
        // Rows with different number of cells
        insta::assert_snapshot!(render(vec![
            vec!["A", "B", "C"],
            vec!["1", "2"], // Missing third cell
            vec!["x"],      // Only one cell
        ]));
    }

    #[test]
    fn test_all_empty_cells() {
        insta::assert_snapshot!(render(vec![vec!["", ""], vec!["", ""],]));
    }

    #[test]
    fn test_whitespace_content() {
        insta::assert_snapshot!(render(vec![
            vec!["Header", "Value"],
            vec!["  spaces  ", "   "],
        ]));
    }

    #[test]
    fn test_special_characters() {
        insta::assert_snapshot!(render(vec![
            vec!["Symbol", "Meaning"],
            vec!["<", "less than"],
            vec![">", "greater than"],
            vec!["&", "ampersand"],
            vec!["\"", "quote"],
        ]));
    }

    #[test]
    fn test_numeric_content() {
        insta::assert_snapshot!(render(vec![
            vec!["ID", "Price", "Quantity"],
            vec!["1001", "$19.99", "150"],
            vec!["1002", "$249.50", "25"],
            vec!["1003", "$5.00", "1000"],
        ]));
    }

    // ==================== Inner tags / inline formatting ====================

    #[test]
    fn test_cell_with_bold_tag() {
        // Bold text in cell - tags should pass through as-is (not rendered)
        insta::assert_snapshot!(render(vec![
            vec!["Feature", "Status"],
            vec!["**Important**", "Done"],
        ]));
    }

    #[test]
    fn test_cell_with_italic_tag() {
        insta::assert_snapshot!(render(vec![
            vec!["Note", "Details"],
            vec!["*emphasis*", "Regular text"],
        ]));
    }

    #[test]
    fn test_cell_with_code_tag() {
        insta::assert_snapshot!(render(vec![
            vec!["Function", "Description"],
            vec!["`render()`", "Renders the output"],
            vec!["`parse()`", "Parses input data"],
        ]));
    }

    #[test]
    fn test_cell_with_link_tag() {
        insta::assert_snapshot!(render(vec![
            vec!["Resource", "URL"],
            vec!["Documentation", "[docs](https://example.com)"],
        ]));
    }

    #[test]
    fn test_cell_with_mixed_tags() {
        insta::assert_snapshot!(render(vec![
            vec!["Item", "Description"],
            vec!["**Bold** and *italic*", "Mixed `code` here"],
            vec!["Normal", "[link](url) text"],
        ]));
    }

    #[test]
    fn test_cell_with_nested_tags() {
        insta::assert_snapshot!(render(vec![
            vec!["Complex", "Content"],
            vec!["***bold italic***", "~~strikethrough~~"],
        ]));
    }

    #[test]
    fn test_header_with_tags() {
        // Tags in header row
        insta::assert_snapshot!(render(vec![
            vec!["**Bold Header**", "`Code Header`"],
            vec!["data1", "data2"],
        ]));
    }

    #[test]
    fn test_cell_with_html_like_tags() {
        // HTML-like content should be preserved
        insta::assert_snapshot!(render(vec![
            vec!["HTML", "Content"],
            vec!["<div>", "element"],
            vec!["<span class='x'>", "styled"],
        ]));
    }

    #[test]
    fn test_long_content_with_tags_wrapping() {
        insta::assert_snapshot!(render_with_width(
            vec![
                vec!["Title", "Content"],
                vec![
                    "Article",
                    "This has **bold** and *italic* and `code` in a long sentence that wraps"
                ],
            ],
            50
        ));
    }

    #[test]
    fn test_unicode_with_tags() {
        insta::assert_snapshot!(render(vec![
            vec!["言語", "説明"],
            vec!["**日本語**", "*Japanese*"],
            vec!["`中文`", "Chinese"],
        ]));
    }

    #[test]
    fn test_real_world_table_with_all_formatting() {
        insta::assert_snapshot!(render(vec![
            vec!["Feature", "Status", "Description", "Link"],
            vec![
                "**Authentication**",
                "✅ `completed`",
                "Implements *JWT-based* authentication with ~~basic~~ **OAuth2** support",
                "[Docs](https://example.com)",
            ],
            vec![
                "**Database Layer**",
                "🚧 `in-progress`",
                "Uses `PostgreSQL` with **Diesel ORM** for *type-safe* queries",
                "[GitHub](https://github.com)",
            ],
            vec![
                "**API Gateway**",
                "⏳ `planned`",
                "RESTful API with `async/await` and ~~synchronous~~ **asynchronous** handlers",
                "[Spec](https://api.example.com)",
            ],
            vec![
                "**Testing**",
                "✅ `completed`",
                "Includes *unit tests*, **integration tests**, and `snapshot testing`",
                "[Coverage](https://coverage.io)",
            ],
            vec![
                "**Deployment**",
                "🚧 `in-progress`",
                "Docker containerization with `K8s` orchestration and **CI/CD** pipeline",
                "[Deploy](https://deploy.com)",
            ],
        ]));
    }
}
