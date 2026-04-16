use std::collections::BTreeMap;

use console::style;
use derive_setters::Setters;
use regex::Regex;

/// RipGrepFormatter formats search results in ripgrep-like style.
#[derive(Clone, Setters)]
#[setters(into, strip_option)]
pub struct GrepFormat {
    lines: Vec<String>,
    regex: Option<Regex>,
}

/// Represents a parsed line from grep-like output format
/// (path:line_num:content)
#[derive(Debug)]
struct ParsedLine<'a> {
    /// File path where the match was found
    path: &'a str,
    /// Line number of the match
    line_num: &'a str,
    /// Content of the matching line
    content: &'a str,
}

impl<'a> ParsedLine<'a> {
    /// Parse a line in the format "path:line_num:content"
    ///
    /// # Arguments
    /// * `line` - The line to parse in the format "path:line_num:content"
    ///
    /// # Returns
    /// * `Some(ParsedLine)` if the line matches the expected format
    /// * `None` if the line is malformed
    fn parse(line: &'a str) -> Option<Self> {
        let parts: Vec<_> = line.split(':').collect();
        if parts.len() != 3 {
            return None;
        }

        let path = parts.first()?.trim();
        let line_num = parts.get(1)?.trim();
        let content = parts.get(2)?.trim();

        // Validate that path and line number parts are not empty
        // and that line number contains only digits
        if path.is_empty() || line_num.is_empty() || !line_num.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }

        Some(Self { path, line_num, content })
    }
}

type Lines<'a> = Vec<(&'a str, &'a str)>;
impl GrepFormat {
    /// Create a new GrepFormat without a specific regex
    pub fn new(lines: Vec<String>) -> Self {
        Self { lines, regex: None }
    }

    /// Collect file entries and determine the maximum line number width
    fn collect_entries<'a>(&'a self) -> (BTreeMap<&'a str, Lines<'a>>, usize) {
        self.lines
            .iter()
            .map(String::as_str)
            .filter_map(ParsedLine::parse)
            .fold((BTreeMap::new(), 0), |(mut entries, max_width), parsed| {
                let new_width = max_width.max(parsed.line_num.len());
                entries
                    .entry(parsed.path)
                    .or_default()
                    .push((parsed.line_num, parsed.content));
                (entries, new_width)
            })
    }

    /// Format a single line with colorization and consistent padding
    fn format_line(&self, num: &str, content: &str, padding: usize) -> String {
        let num = style(format!("{num:>padding$}: ")).dim();

        // Format the content with highlighting if regex is available
        let line = match self.regex {
            Some(ref regex) => regex.find(content).map_or_else(
                || content.to_string(),
                |mat| {
                    format!(
                        "{}{}{}",
                        content.get(..mat.start()).unwrap_or(""),
                        style(content.get(mat.start()..mat.end()).unwrap_or(""))
                            .yellow()
                            .bold(),
                        content.get(mat.end()..).unwrap_or("")
                    )
                },
            ),
            None => content.to_string(),
        };

        format!("{num}{line}\n")
    }

    /// Format a group of lines for a single file
    fn format_file_group(
        &self,
        path: &str,
        group: Vec<(&str, &str)>,
        max_num_width: usize,
    ) -> String {
        let file_header = style(path).cyan();
        let formatted_lines = group
            .into_iter()
            .map(|(num, content)| self.format_line(num, content, max_num_width))
            .collect::<String>();
        format!("{file_header}\n{formatted_lines}")
    }

    /// Handle raw file paths (entries without line:content format)
    fn format_raw_paths(&self) -> String {
        // Collect and format all raw file paths
        let formatted_paths: Vec<_> = self
            .lines
            .iter()
            .map(|line| format!("{}", style(line).cyan()))
            .collect();

        // Join with newlines
        formatted_paths.join("\n")
    }

    /// Format search results with colorized output grouped by path
    pub fn format(&self) -> String {
        if self.lines.is_empty() {
            return String::new();
        }

        // First check if we have any valid grep format entries
        let has_valid_entries = self
            .lines
            .iter()
            .any(|line| ParsedLine::parse(line).is_some());

        // If no valid grep format entries found, treat all lines as raw file paths
        if !has_valid_entries {
            return self.format_raw_paths();
        }

        // First pass: collect entries and find max width
        let (entries, max_num_width) = self.collect_entries();

        // Print the results on separate lines
        let formatted_entries: Vec<_> = entries
            .into_iter()
            .map(|(path, group)| self.format_file_group(path, group, max_num_width))
            .collect();

        // Join all results with newlines
        formatted_entries.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::{Display, Formatter};

    use insta::assert_snapshot;

    use super::*;

    /// Specification for a grep format test case
    #[derive(Debug)]
    struct GrepSpec {
        description: String,
        input: Vec<String>,
        output: String,
    }

    impl GrepSpec {
        /// Create a new test specification with computed fields
        fn new(description: &str, input: Vec<&str>, pattern: Option<&str>) -> Self {
            let input: Vec<String> = input.iter().map(|s| s.to_string()).collect();

            // Generate the formatted output
            let formatter = match pattern {
                Some(pattern) => GrepFormat::new(input.clone()).regex(Regex::new(pattern).unwrap()),
                None => GrepFormat::new(input.clone()),
            };

            let output = strip_ansi_escapes::strip_str(formatter.format()).to_string();

            Self { description: description.to_string(), input, output }
        }
    }

    impl Display for GrepSpec {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            writeln!(f, "\n[{}]", self.description)?;
            writeln!(f, "[RAW]")?;
            writeln!(f, "{}", self.input.join("\n"))?;
            writeln!(f, "[FMT]")?;
            writeln!(f, "{}", self.output)
        }
    }

    #[derive(Default, Debug)]
    struct GrepSuite(Vec<GrepSpec>);

    impl GrepSuite {
        fn add(&mut self, description: &str, input: Vec<&str>, pattern: Option<&str>) {
            self.0.push(GrepSpec::new(description, input, pattern));
        }
    }

    impl Display for GrepSuite {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            for spec in &self.0 {
                writeln!(f, "{spec}")?;
            }
            Ok(())
        }
    }

    #[test]
    fn test_combined_grep_suite() {
        let mut suite = GrepSuite::default();

        suite.add(
            "Basic single file with two matches",
            vec!["file.txt:1:first match", "file.txt:2:second match"],
            Some("match"),
        );

        suite.add(
            "Multiple files with various matches",
            vec![
                "file1.txt:1:match in file1",
                "file2.txt:1:first match in file2",
                "file2.txt:2:second match in file2",
                "file3.txt:1:match in file3",
            ],
            Some("file"),
        );

        suite.add(
            "File with varying line number widths",
            vec![
                "file.txt:1:first line",
                "file.txt:5:fifth line",
                "file.txt:10:tenth line",
                "file.txt:100:hundredth line",
            ],
            Some("line"),
        );

        suite.add(
            "Mix of valid and invalid input lines",
            vec![
                "file.txt:1:valid match",
                "malformed line without separator",
                "file.txt:2:another valid match",
            ],
            Some("match"),
        );

        suite.add("Empty input vector", vec![], None);

        suite.add(
            "Input with special characters and formatting",
            vec![
                "path/to/file.txt:1:contains 🦀 rust",
                "path/to/file.txt:2:has\ttabs\tand\tspaces",
                "path/to/file.txt:3:contains\nnewlines",
            ],
            Some("contains"),
        );

        suite.add(
            "Multiple files with same line numbers",
            vec![
                "test1.rs:10:fn test1()",
                "test2.rs:10:fn test2()",
                "test3.rs:10:fn test3()",
            ],
            Some("fn"),
        );

        suite.add(
            "Content with full-width unicode characters",
            vec![
                "test.txt:1:Contains 你好 characters",
                "test.txt:2:More UTF-8 ありがとう here",
            ],
            Some("Contains"),
        );

        // New test cases for testing without regex
        suite.add(
            "Without regex - Basic single file with two matches",
            vec!["file.txt:1:first match", "file.txt:2:second match"],
            None,
        );

        suite.add(
            "Without regex - Multiple files with various patterns",
            vec![
                "file1.txt:1:regex pattern in file1",
                "file2.txt:1:another pattern in file2",
                "file2.txt:2:different pattern in file2",
            ],
            None,
        );

        assert_snapshot!(suite);
    }

    #[test]
    fn test_with_and_without_regex() {
        let lines = vec!["a/b/c.md".to_string(), "p/q/r.rs".to_string()];

        // Test without regex
        let grep = GrepFormat::new(lines);
        let output = strip_ansi_escapes::strip_str(grep.format()).to_string();

        assert!(output.contains("c.md"));
        assert!(output.contains("r.rs"));
    }
}
