use reedline::Span;

pub struct SearchTerm {
    line: String,
    position: usize,
}

impl SearchTerm {
    pub fn new(line: &str, position: usize) -> Self {
        if position > line.len() {
            panic!(
                "Position {position} is out of bounds: string '{line}' (length: {})",
                line.len()
            );
        }
        Self { line: line.to_string(), position }
    }

    /// Get the search term from the line based on '@' marker or cursor position
    ///
    /// If '@' marker is present, returns the word following it.
    /// Otherwise, returns the word at the cursor position.
    /// If no word is found, returns None.
    pub fn process(&self) -> Option<TermResult<'_>> {
        // Ensure position is on a UTF-8 character boundary to prevent panics
        let safe_position = if self.line.is_char_boundary(self.position) {
            self.position
        } else {
            // Find the nearest lower character boundary
            (0..self.position)
                .rev()
                .find(|&i| self.line.is_char_boundary(i))
                .unwrap_or(0)
        };

        let prefix = self.line.get(..safe_position)?;
        let at_pos = prefix.rfind('@')?;
        let start_pos = at_pos + 1;
        let term = self.line.get(start_pos..safe_position)?;

        Some(TermResult { span: Span::new(start_pos, safe_position), term })
    }
}

#[derive(Debug)]
pub struct TermResult<'a> {
    pub span: Span,
    pub term: &'a str,
}

#[cfg(test)]
mod tests {
    use insta::assert_debug_snapshot;

    use super::SearchTerm;

    impl SearchTerm {
        fn test(line: &str) -> Vec<TermSpec> {
            // Test at each valid character boundary position, starting from 1
            (1..=line.len())
                .filter(|&pos| line.is_char_boundary(pos))
                .map(|pos| {
                    let input = SearchTerm::new(line, pos);
                    let output = input.process();
                    let (a, b) = line.split_at(pos);

                    TermSpec {
                        pos,
                        input: format!("{a}[{b}"),
                        output: output.as_ref().map(|term| term.term.to_string()),
                        span_start: output.as_ref().map(|term| term.span.start),
                        span_end: output.as_ref().map(|term| term.span.end),
                    }
                })
                .collect()
        }
    }

    #[derive(Debug)]
    #[allow(dead_code)] // Used to generate test snapshots
    struct TermSpec {
        input: String,
        output: Option<String>,
        span_start: Option<usize>,
        span_end: Option<usize>,
        pos: usize,
    }

    #[test]
    fn test_marker_based_search() {
        let results = SearchTerm::test("@abc @def ghi@");
        assert_debug_snapshot!(results);
    }

    #[test]
    fn test_marker_based_search_chinese() {
        let results = SearchTerm::test("@你好 @世界 测试@");
        assert_debug_snapshot!(results);
    }

    #[test]
    fn test_marker_based_search_mixed_chinese_english() {
        let results = SearchTerm::test("@hello @世界 test@中文");
        assert_debug_snapshot!(results);
    }

    #[test]
    fn test_marker_based_search_chinese_with_spaces() {
        let results = SearchTerm::test("@中 文 @测试");
        assert_debug_snapshot!(results);
    }

    #[test]
    fn test_marker_based_search_emoji() {
        let results = SearchTerm::test("@🚀 @🌟 emoji@");
        assert_debug_snapshot!(results);
    }
}
