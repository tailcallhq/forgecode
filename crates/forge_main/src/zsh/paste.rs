//! Pasted-text formatting utilities.
//!
//! When the terminal sends a bracketed-paste event (e.g. from a drag-and-drop),
//! this module checks whether the pasted text is an existing file path and,
//! if so, wraps it in `@[...]` syntax so the user sees the reference
//! immediately in the input field.

use std::path::Path;

/// Transforms pasted text by wrapping bare file paths in `@[...]` syntax.
///
/// Called when a bracketed-paste event is received. The pasted content is
/// normalised (CRLF to LF) and then checked for file paths. If the entire
/// paste (after stripping whitespace/quotes) is a single existing absolute
/// path it gets wrapped directly -- this handles paths with spaces. Otherwise
/// the text is scanned token-by-token for quoted or unquoted absolute paths.
///
/// Already-wrapped `@[...]` references and non-existent paths are left
/// untouched.
pub fn wrap_pasted_text(pasted: &str) -> String {
    let normalised = pasted.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = normalised.trim();

    // If the whole paste is empty, just return normalised form
    if trimmed.is_empty() {
        return normalised;
    }

    // Fast path: the entire paste is a single path (possibly quoted).
    // This is the most common drag-and-drop case and correctly handles
    // paths that contain spaces or backslash-escaped spaces.
    let unquoted = strip_surrounding_quotes(trimmed);
    if let Some(resolved) = resolve_file_path(unquoted) {
        // Reconstruct with the same leading/trailing whitespace the
        // original normalised string had.
        let leading = &normalised[..normalised.len() - normalised.trim_start().len()];
        let trailing = &normalised[normalised.trim_end().len()..];
        return format!("{leading}@[{resolved}]{trailing}");
    }

    // Scan token by token, wrapping any absolute paths that exist on disk
    wrap_tokens(&normalised)
}

/// Strips surrounding single or double quotes that some terminals add
/// when dragging files with spaces in their names.
fn strip_surrounding_quotes(s: &str) -> &str {
    if s.len() < 2 {
        return s;
    }
    if (s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')) {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Removes backslash escapes from a string (e.g. `\ ` becomes ` `).
///
/// Many terminals (Ghostty, iTerm2, etc.) backslash-escape spaces when
/// drag-and-dropping file paths, producing strings like
/// `/path/my\ folder/file.txt`. This helper un-escapes them so the path
/// can be resolved against the filesystem.
///
/// Returns `None` if no backslash escapes were found (i.e. the input is
/// already clean), allowing callers to skip redundant `is_file()` checks.
fn unescape_backslashes(s: &str) -> Option<String> {
    if !s.contains('\\') {
        return None;
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            // Take the next char literally, or keep the backslash if at end
            if let Some(next) = chars.next() {
                out.push(next);
            } else {
                out.push(c);
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Checks whether `candidate` resolves to an existing absolute file or
/// directory path.
///
/// Tries the raw string first, then falls back to un-escaping backslashes
/// (for terminals that send `/path/my\ file.txt`). Returns the resolved
/// clean path on success, or `None` if no match was found.
fn resolve_file_path(candidate: &str) -> Option<String> {
    let path = Path::new(candidate);
    if path.is_absolute() && path.exists() {
        return Some(candidate.to_string());
    }
    // Try un-escaping backslashes (e.g. Ghostty sends `/path/my\ file.txt`)
    if let Some(unescaped) = unescape_backslashes(candidate) {
        let path = Path::new(&unescaped);
        if path.is_absolute() && path.exists() {
            return Some(unescaped);
        }
    }
    None
}

/// Finds the end of a token in `input`, treating `\<char>` as an escaped
/// character that is part of the token (not a boundary).
///
/// Returns the byte offset of the first unescaped whitespace character, or
/// the length of `input` if no unescaped whitespace is found.
fn find_token_end(input: &str) -> usize {
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            // Skip escaped character
            i += 2;
        } else if (bytes[i] as char).is_whitespace() {
            return i;
        } else {
            i += 1;
        }
    }
    input.len()
}

/// Walks through `input` token-by-token and wraps absolute file paths.
///
/// Handles both unquoted tokens (split on whitespace) and quoted strings
/// (single or double quotes) so that paths containing spaces are kept
/// together as a single token.
fn wrap_tokens(input: &str) -> String {
    let mut result = String::with_capacity(input.len() + 32);
    let mut remaining = input;

    while !remaining.is_empty() {
        // Preserve leading whitespace
        let ws_end = remaining
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(remaining.len());
        result.push_str(&remaining[..ws_end]);
        remaining = &remaining[ws_end..];

        if remaining.is_empty() {
            break;
        }

        // Skip already-wrapped @[...] references
        if remaining.starts_with("@[")
            && let Some(close) = remaining.find(']')
        {
            result.push_str(&remaining[..=close]);
            remaining = &remaining[close + 1..];
            continue;
        }

        // If the token starts with a quote, consume everything up to the
        // matching closing quote so that paths with spaces stay together.
        let first_char = remaining.as_bytes()[0];
        if first_char == b'\'' || first_char == b'"' {
            let quote = first_char as char;
            if let Some(close) = remaining[1..].find(quote) {
                let token_end = close + 2; // include both quotes
                let token = &remaining[..token_end];
                let clean = strip_surrounding_quotes(token);
                if let Some(resolved) = resolve_file_path(clean) {
                    result.push_str(&format!("@[{}]", resolved));
                } else {
                    result.push_str(token);
                }
                remaining = &remaining[token_end..];
                continue;
            }
        }

        // Extract the next token, treating backslash-escaped whitespace
        // (e.g. `\ `) as part of the token.  This handles terminals like
        // Ghostty that send `/path/my\ file.txt` for drag-and-drop.
        let token_end = find_token_end(remaining);
        let token = &remaining[..token_end];

        if let Some(resolved) = resolve_file_path(token) {
            result.push_str(&format!("@[{}]", resolved));
        } else {
            result.push_str(token);
        }

        remaining = &remaining[token_end..];
    }

    result
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_wrap_pasted_text_no_paths() {
        let fixture = "hello world";
        let actual = wrap_pasted_text(fixture);
        let expected = "hello world";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_already_wrapped() {
        let fixture = "check @[/usr/bin/env]";
        let actual = wrap_pasted_text(fixture);
        let expected = "check @[/usr/bin/env]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_existing_file() {
        // /usr/bin/env exists on macOS/Linux
        let fixture = "look at /usr/bin/env please";
        let actual = wrap_pasted_text(fixture);
        let expected = "look at @[/usr/bin/env] please";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_nonexistent_path_untouched() {
        let fixture = "look at /nonexistent/path/file.rs please";
        let actual = wrap_pasted_text(fixture);
        let expected = "look at /nonexistent/path/file.rs please";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_bare_path_only() {
        // Just a bare path (typical drag-and-drop result)
        // /usr/bin/env is a real file, so it should be wrapped
        let fixture = "/usr/bin/env";
        let actual = wrap_pasted_text(fixture);
        let expected = "@[/usr/bin/env]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_bare_path_nonexistent() {
        let fixture = "/nonexistent/path/file.rs";
        let actual = wrap_pasted_text(fixture);
        let expected = "/nonexistent/path/file.rs";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_with_text_before() {
        let fixture = "analyze /usr/bin/env";
        let actual = wrap_pasted_text(fixture);
        let expected = "analyze @[/usr/bin/env]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_preserves_whitespace() {
        let fixture = "hello  world";
        let actual = wrap_pasted_text(fixture);
        let expected = "hello  world";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_mixed_existing_and_nonexistent() {
        let fixture = "check /usr/bin/env and /nonexistent/foo.rs";
        let actual = wrap_pasted_text(fixture);
        let expected = "check @[/usr/bin/env] and /nonexistent/foo.rs";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_crlf_normalised() {
        let fixture = "/usr/bin/env\r\n";
        let actual = wrap_pasted_text(fixture);
        let expected = "@[/usr/bin/env]\n";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_single_quoted_path() {
        let fixture = "'/usr/bin/env'";
        let actual = wrap_pasted_text(fixture);
        let expected = "@[/usr/bin/env]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_strip_surrounding_quotes_single() {
        let actual = strip_surrounding_quotes("'/some/path'");
        let expected = "/some/path";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_strip_surrounding_quotes_double() {
        let actual = strip_surrounding_quotes("\"/some/path\"");
        let expected = "/some/path";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_strip_surrounding_quotes_none() {
        let actual = strip_surrounding_quotes("/some/path");
        let expected = "/some/path";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_strip_surrounding_quotes_single_char() {
        let actual = strip_surrounding_quotes("'");
        let expected = "'";
        assert_eq!(actual, expected);
    }

    // -- Tests for paths with spaces -----------------------------------------

    /// Helper that creates a temp directory containing a file at the given
    /// relative path (which may include spaces) and returns the absolute path
    /// to that file along with the `TempDir` guard to keep it alive.
    fn create_file_with_spaces(relative: &str) -> (String, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join(relative);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&file_path, "test").unwrap();
        (file_path.to_string_lossy().into_owned(), dir)
    }

    #[test]
    fn test_wrap_pasted_text_bare_path_with_spaces() {
        let (path, _dir) = create_file_with_spaces("my file.txt");
        let actual = wrap_pasted_text(&path);
        let expected = format!("@[{path}]");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_single_quoted_path_with_spaces() {
        let (path, _dir) = create_file_with_spaces("my file.txt");
        let fixture = format!("'{path}'");
        let actual = wrap_pasted_text(&fixture);
        let expected = format!("@[{path}]");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_double_quoted_path_with_spaces() {
        let (path, _dir) = create_file_with_spaces("my file.txt");
        let fixture = format!("\"{path}\"");
        let actual = wrap_pasted_text(&fixture);
        let expected = format!("@[{path}]");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_path_with_spaces_in_directory() {
        let (path, _dir) = create_file_with_spaces("my folder/file.txt");
        let actual = wrap_pasted_text(&path);
        let expected = format!("@[{path}]");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_path_with_spaces_trailing_newline() {
        let (path, _dir) = create_file_with_spaces("my file.txt");
        let fixture = format!("{path}\n");
        let actual = wrap_pasted_text(&fixture);
        let expected = format!("@[{path}]\n");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_path_with_spaces_crlf() {
        let (path, _dir) = create_file_with_spaces("my file.txt");
        let fixture = format!("{path}\r\n");
        let actual = wrap_pasted_text(&fixture);
        let expected = format!("@[{path}]\n");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_quoted_path_with_spaces_in_sentence() {
        let (path, _dir) = create_file_with_spaces("my file.txt");
        let fixture = format!("check '{path}' please");
        let actual = wrap_pasted_text(&fixture);
        let expected = format!("check @[{path}] please");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_nonexistent_path_with_spaces() {
        let fixture = "/nonexistent/path with spaces/file.txt";
        let actual = wrap_pasted_text(fixture);
        let expected = "/nonexistent/path with spaces/file.txt";
        assert_eq!(actual, expected);
    }

    // -- Tests for backslash-escaped paths -----------------------------------

    #[test]
    fn test_wrap_pasted_text_backslash_escaped_spaces() {
        // Terminals like Ghostty send /path/my\ file.txt for drag-and-drop
        let (path, _dir) = create_file_with_spaces("my file.txt");
        let escaped = path.replace(' ', "\\ ");
        let actual = wrap_pasted_text(&escaped);
        let expected = format!("@[{path}]");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_backslash_escaped_spaces_in_directory() {
        let (path, _dir) = create_file_with_spaces("my folder/file.txt");
        let escaped = path.replace(' ', "\\ ");
        let actual = wrap_pasted_text(&escaped);
        let expected = format!("@[{path}]");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_backslash_escaped_nonexistent() {
        let fixture = "/nonexistent/my\\ folder/file.txt";
        let actual = wrap_pasted_text(fixture);
        let expected = "/nonexistent/my\\ folder/file.txt";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_backslash_escaped_in_sentence() {
        let (path, _dir) = create_file_with_spaces("my file.txt");
        let escaped = path.replace(' ', "\\ ");
        let fixture = format!("check {escaped} please");
        let actual = wrap_pasted_text(&fixture);
        let expected = format!("check @[{path}] please");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_unescape_backslashes_spaces() {
        let actual = unescape_backslashes("/path/my\\ file.txt");
        let expected = Some("/path/my file.txt".to_string());
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_unescape_backslashes_no_escapes() {
        let actual = unescape_backslashes("/path/file.txt");
        assert_eq!(actual, None);
    }

    #[test]
    fn test_unescape_backslashes_trailing_backslash() {
        let actual = unescape_backslashes("/path/file\\");
        let expected = Some("/path/file\\".to_string());
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_resolve_file_path_plain() {
        let actual = resolve_file_path("/usr/bin/env");
        assert_eq!(actual, Some("/usr/bin/env".to_string()));
    }

    #[test]
    fn test_resolve_file_path_escaped() {
        let (path, _dir) = create_file_with_spaces("my file.txt");
        let escaped = path.replace(' ', "\\ ");
        let actual = resolve_file_path(&escaped);
        assert_eq!(actual, Some(path));
    }

    #[test]
    fn test_resolve_file_path_nonexistent() {
        let actual = resolve_file_path("/nonexistent/file.txt");
        assert_eq!(actual, None);
    }

    #[test]
    fn test_resolve_file_path_directory() {
        // /tmp is a real directory on macOS/Linux
        let actual = resolve_file_path("/tmp");
        assert_eq!(actual, Some("/tmp".to_string()));
    }

    #[test]
    fn test_wrap_pasted_text_directory_path() {
        // /tmp is a real directory, so it should be wrapped
        let fixture = "/tmp";
        let actual = wrap_pasted_text(fixture);
        let expected = "@[/tmp]";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_directory_in_sentence() {
        let fixture = "look at /tmp please";
        let actual = wrap_pasted_text(fixture);
        let expected = "look at @[/tmp] please";
        assert_eq!(actual, expected);
    }

    // -- Tests for VSCode-style drag-and-drop (path sent via sendText) -------

    #[test]
    fn test_wrap_pasted_text_vscode_quoted_path() {
        // VSCode's preparePathForShell may single-quote paths with spaces
        let (path, _dir) = create_file_with_spaces("my file.txt");
        let fixture = format!("'{path}'");
        let actual = wrap_pasted_text(&fixture);
        let expected = format!("@[{path}]");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_wrap_pasted_text_vscode_plain_path() {
        // VSCode sends a plain path for files without spaces
        let fixture = "/usr/bin/env";
        let actual = wrap_pasted_text(fixture);
        let expected = "@[/usr/bin/env]";
        assert_eq!(actual, expected);
    }
}
