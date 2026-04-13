use nu_ansi_term::{Color, Style};
use reedline::{Highlighter, StyledText};

/// Syntax highlighter for the forge readline prompt.
///
/// Applies visual styles to recognised input patterns as the user types:
/// - Commands (`:foo` or `/foo` for backward compatibility) are rendered in
///   yellow bold.
/// - File mentions (`@[path]`) are rendered in cyan bold.
/// - Shell pass-through commands (`!cmd`) are rendered in magenta.
/// - All other text is rendered in the default terminal style.
pub struct ForgeHighlighter;

impl Highlighter for ForgeHighlighter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let mut styled = StyledText::new();

        if line.is_empty() {
            return styled;
        }

        // Command: highlight the command token (e.g. `:compact` or `/compact` for
        // compat) in yellow bold, then the remainder (arguments) without
        // special styling.
        if line.starts_with('/') || line.starts_with(':') {
            let end = line.find(|c: char| c.is_whitespace()).unwrap_or(line.len());
            styled.push((
                Style::new().bold().fg(Color::Yellow),
                line[..end].to_string(),
            ));
            if end < line.len() {
                styled.push((Style::new(), line[end..].to_string()));
            }
            return styled;
        }

        // Shell pass-through: `!<cmd>` rendered in magenta.
        if line.starts_with('!') {
            styled.push((Style::new().fg(Color::Magenta), line.to_string()));
            return styled;
        }

        // General message text — scan for `@[...]` file mentions and colour them cyan
        // bold.
        highlight_mentions(line, &mut styled);

        styled
    }
}

/// Walk through `line` and emit styled segments, colouring every `@[...]`
/// mention (matching the ZSH pattern `@\[[^]]*\]`) in cyan bold and leaving
/// surrounding text unstyled.
///
/// Mirrors `ZSH_HIGHLIGHT_PATTERNS+=('@\[[^]]#\]' 'fg=cyan,bold')` exactly:
/// - Requires `@[` opener.
/// - Matches zero or more non-`]` characters inside the brackets.
/// - Requires closing `]` — unterminated tags are left unstyled.
fn highlight_mentions(line: &str, styled: &mut StyledText) {
    let mut remaining = line;

    while !remaining.is_empty() {
        // Find the next `@[` opener.
        match remaining.find("@[") {
            None => {
                // No more mentions — emit the rest as plain text.
                styled.push((Style::new(), remaining.to_string()));
                break;
            }
            Some(start) => {
                // Emit any plain text before the `@[`.
                if start > 0 {
                    styled.push((Style::new(), remaining[..start].to_string()));
                }

                // `after_open` starts at `@[`.
                let after_open = &remaining[start..];

                // Look for a `]` that is not immediately after `@[`.
                // The ZSH pattern `[^]]#` matches zero-or-more non-`]` chars,
                // so `@[]` (empty brackets) also qualifies.
                // We search for `]` starting from position 2 (after `@[`).
                match after_open[2..].find(']') {
                    None => {
                        // No closing `]` — emit `@[` and the rest as plain text
                        // to match ZSH behaviour (unterminated tag = no highlight).
                        styled.push((Style::new(), after_open.to_string()));
                        break;
                    }
                    Some(rel_close) => {
                        // Absolute position of `]` within `after_open`.
                        let close = 2 + rel_close;
                        // Emit `@[...]` in cyan bold (inclusive of both brackets).
                        let mention = &after_open[..=close];
                        styled.push((Style::new().bold().fg(Color::Cyan), mention.to_string()));
                        remaining = &after_open[close + 1..];
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(styled: &StyledText) -> String {
        styled.buffer.iter().map(|(_, s)| s.as_str()).collect()
    }

    fn styles(styled: &StyledText) -> Vec<Style> {
        styled.buffer.iter().map(|(style, _)| *style).collect()
    }

    #[test]
    fn test_slash_command_highlighted() {
        let fixture = ForgeHighlighter;
        let actual = fixture.highlight("/compact", 0);
        assert_eq!(render(&actual), "/compact");
        assert_eq!(styles(&actual)[0], Style::new().bold().fg(Color::Yellow));
    }

    #[test]
    fn test_colon_command_highlighted() {
        let fixture = ForgeHighlighter;
        let actual = fixture.highlight(":compact", 0);
        assert_eq!(render(&actual), ":compact");
        assert_eq!(styles(&actual)[0], Style::new().bold().fg(Color::Yellow));
    }

    #[test]
    fn test_colon_command_with_args() {
        let fixture = ForgeHighlighter;
        let actual = fixture.highlight(":commit some message", 0);
        let parts: Vec<_> = actual
            .buffer
            .iter()
            .map(|(s, t)| (*s, t.as_str()))
            .collect();
        assert_eq!(parts[0], (Style::new().bold().fg(Color::Yellow), ":commit"));
        assert_eq!(parts[1].1, " some message");
    }

    #[test]
    fn test_slash_command_with_args() {
        let fixture = ForgeHighlighter;
        let actual = fixture.highlight("/commit some message", 0);
        let parts: Vec<_> = actual
            .buffer
            .iter()
            .map(|(s, t)| (*s, t.as_str()))
            .collect();
        assert_eq!(parts[0], (Style::new().bold().fg(Color::Yellow), "/commit"));
        assert_eq!(parts[1].1, " some message");
    }

    #[test]
    fn test_shell_command_highlighted() {
        let fixture = ForgeHighlighter;
        let actual = fixture.highlight("!ls -la", 0);
        assert_eq!(render(&actual), "!ls -la");
        assert_eq!(styles(&actual)[0], Style::new().fg(Color::Magenta));
    }

    #[test]
    fn test_file_mention_highlighted() {
        let fixture = ForgeHighlighter;
        let actual = fixture.highlight("explain @[src/main.rs] please", 0);
        let parts: Vec<_> = actual
            .buffer
            .iter()
            .map(|(s, t)| (*s, t.as_str()))
            .collect();
        assert_eq!(parts[0], (Style::new(), "explain "));
        assert_eq!(
            parts[1],
            (Style::new().bold().fg(Color::Cyan), "@[src/main.rs]")
        );
        assert_eq!(parts[2], (Style::new(), " please"));
    }

    #[test]
    fn test_multiple_file_mentions() {
        let fixture = ForgeHighlighter;
        let actual = fixture.highlight("@[a.rs] and @[b.rs]", 0);
        let texts: Vec<_> = actual.buffer.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(texts, vec!["@[a.rs]", " and ", "@[b.rs]"]);
        assert_eq!(actual.buffer[0].0, Style::new().bold().fg(Color::Cyan));
        assert_eq!(actual.buffer[2].0, Style::new().bold().fg(Color::Cyan));
    }

    #[test]
    fn test_plain_text_unstyled() {
        let fixture = ForgeHighlighter;
        let actual = fixture.highlight("hello world", 0);
        assert_eq!(render(&actual), "hello world");
        assert_eq!(styles(&actual)[0], Style::new());
    }

    #[test]
    fn test_empty_input() {
        let fixture = ForgeHighlighter;
        let actual = fixture.highlight("", 0);
        assert!(actual.buffer.is_empty());
    }

    // --- ZSH parity tests ---

    #[test]
    fn test_file_mention_with_line_range() {
        // @[path:start:end] — same as ZSH pattern, content inside can contain colons
        let fixture = ForgeHighlighter;
        let actual = fixture.highlight("@[src/main.rs:10:20]", 0);
        assert_eq!(render(&actual), "@[src/main.rs:10:20]");
        assert_eq!(styles(&actual)[0], Style::new().bold().fg(Color::Cyan));
    }

    #[test]
    fn test_file_mention_with_symbol() {
        // @[path#symbol] — content inside can contain `#`
        let fixture = ForgeHighlighter;
        let actual = fixture.highlight("check @[src/lib.rs#my_fn] here", 0);
        let parts: Vec<_> = actual
            .buffer
            .iter()
            .map(|(s, t)| (*s, t.as_str()))
            .collect();
        assert_eq!(
            parts[1],
            (Style::new().bold().fg(Color::Cyan), "@[src/lib.rs#my_fn]")
        );
    }

    #[test]
    fn test_unterminated_mention_not_highlighted() {
        // ZSH requires closing `]`, so unterminated tags are plain text.
        let fixture = ForgeHighlighter;
        let actual = fixture.highlight("see @[src/main.rs", 0);
        assert_eq!(render(&actual), "see @[src/main.rs");
        // No cyan segment
        assert!(
            styles(&actual)
                .iter()
                .all(|s| *s != Style::new().bold().fg(Color::Cyan))
        );
    }

    #[test]
    fn test_bare_at_sign_not_highlighted() {
        // A bare `@word` (no brackets) is plain text — matches ZSH pattern behaviour.
        let fixture = ForgeHighlighter;
        let actual = fixture.highlight("email@example.com", 0);
        assert_eq!(render(&actual), "email@example.com");
        assert_eq!(styles(&actual)[0], Style::new());
    }
}
