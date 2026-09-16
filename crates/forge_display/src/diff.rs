use std::fmt;

use console::{Style, style};
use similar::{ChangeTag, TextDiff};

struct Line {
    index: Option<usize>,
    width: usize,
}

impl Line {
    fn new(index: Option<usize>, width: usize) -> Self {
        Self { index, width }
    }
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.index {
            None => write!(f, "{:width$}", "", width = self.width),
            Some(idx) => write!(f, "{:<width$}", idx + 1, width = self.width),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiffResult {
    result: String,
    lines_added: u64,
    lines_removed: u64,
}

impl DiffResult {
    pub fn diff(&self) -> &str {
        &self.result
    }

    pub fn lines_added(&self) -> u64 {
        self.lines_added
    }

    pub fn lines_removed(&self) -> u64 {
        self.lines_removed
    }
}

pub struct DiffFormat;

impl DiffFormat {
    pub fn format(old: &str, new: &str) -> DiffResult {
        let diff = TextDiff::from_lines(old, new);
        let ops = diff.grouped_ops(3);
        let mut output = String::new();

        let mut lines_added = 0;
        let mut lines_removed = 0;

        if ops.is_empty() {
            output.push_str(&format!("{}\n", style("No changes applied").dim()));

            return DiffResult { result: output, lines_added, lines_removed };
        }

        // First pass: Calculate dynamic width based on max line numbers in
        // actual changes
        let mut max_line_number = 0;
        for group in &ops {
            for op in group {
                for change in diff.iter_inline_changes(op) {
                    if let Some(old_idx) = change.old_index() {
                        max_line_number = max_line_number.max(old_idx + 1);
                    }
                    if let Some(new_idx) = change.new_index() {
                        max_line_number = max_line_number.max(new_idx + 1);
                    }
                }
            }
        }
        let width = if max_line_number == 0 {
            1
        } else {
            (max_line_number as f64).log10().floor() as usize + 1
        };

        // Second pass: Format the output
        for (idx, group) in ops.iter().enumerate() {
            if idx > 0 {
                output.push_str(&format!("{}\n", style("...").dim()));
            }
            for op in group {
                for change in diff.iter_inline_changes(op) {
                    let (sign, s) = match change.tag() {
                        ChangeTag::Delete => {
                            lines_removed += 1;
                            ("-", Style::new().red())
                        }
                        ChangeTag::Insert => {
                            lines_added += 1;
                            ("+", Style::new().yellow())
                        }
                        ChangeTag::Equal => (" ", Style::new().dim()),
                    };

                    output.push_str(&format!(
                        "{} {} |{}",
                        style(Line::new(change.old_index(), width)).dim(),
                        style(Line::new(change.new_index(), width)).dim(),
                        s.apply_to(sign),
                    ));

                    for (_, value) in change.iter_strings_lossy() {
                        output.push_str(&format!("{}", s.apply_to(value)));
                    }
                    if change.missing_newline() {
                        output.push('\n');
                    }
                }
            }
        }

        DiffResult { result: output, lines_added, lines_removed }
    }
}

#[cfg(test)]
mod tests {
    use console::strip_ansi_codes;
    use insta::assert_snapshot;

    use super::*;

    #[test]
    fn test_color_output() {
        let old = "Hello World\nThis is a test\nThird line\nFourth line";
        let new = "Hello World\nThis is a modified test\nNew line\nFourth line";
        let diff = DiffFormat::format(old, new);
        let diff_str = diff.diff();
        assert_eq!(diff.lines_added(), 2);
        assert_eq!(diff.lines_removed(), 2);
        eprintln!("\nColor Output Test:\n{diff_str}");
    }

    #[test]
    fn test_diff_printer_no_differences() {
        let content = "line 1\nline 2\nline 3";
        let diff = DiffFormat::format(content, content);
        assert_eq!(diff.lines_added(), 0);
        assert_eq!(diff.lines_removed(), 0);
        assert!(diff.diff().contains("No changes applied"));
    }

    #[test]
    fn test_file_source() {
        let old = "line 1\nline 2\nline 3\nline 4\nline 5";
        let new = "line 1\nline 2\nline 3";
        let diff = DiffFormat::format(old, new);
        let clean_diff = strip_ansi_codes(diff.diff());
        assert_eq!(diff.lines_added(), 1);
        assert_eq!(diff.lines_removed(), 3);
        assert_snapshot!(clean_diff);
    }

    #[test]
    fn test_diff_printer_simple_diff() {
        let old = "line 1\nline 2\nline 3\nline 5\nline 6\nline 7\nline 8\nline 9";
        let new = "line 1\nmodified line\nline 3\nline 5\nline 6\nline 7\nline 8\nline 9";
        let diff = DiffFormat::format(old, new);
        let clean_diff = strip_ansi_codes(diff.diff());
        assert_eq!(diff.lines_added(), 1);
        assert_eq!(diff.lines_removed(), 1);
        assert_snapshot!(clean_diff);
    }

    #[test]
    fn test_dynamic_width_with_large_line_numbers() {
        // Test with 100+ lines to verify width calculation
        let old_lines = (1..=150).map(|i| format!("line {i}")).collect::<Vec<_>>();
        let mut new_lines = old_lines.clone();
        new_lines[99] = "modified line 100".to_string();

        let old = old_lines.join("\n");
        let new = new_lines.join("\n");
        let diff = DiffFormat::format(&old, &new);
        let clean_diff = strip_ansi_codes(diff.diff());

        // With 150 lines, width should be 3 (for numbers like "100")
        // Verify the format includes proper spacing
        assert!(clean_diff.contains("100"));
        assert_eq!(diff.lines_added(), 1);
    }

    #[test]
    fn test_width_based_on_diff_not_file_size() {
        // Large file but diff only at the beginning
        let old_lines = (1..=1000).map(|i| format!("line {i}")).collect::<Vec<_>>();
        let mut new_lines = old_lines.clone();
        new_lines[4] = "modified line 5".to_string(); // Only change line 5

        let old = old_lines.join("\n");
        let new = new_lines.join("\n");
        let diff = DiffFormat::format(&old, &new);
        let clean_diff = strip_ansi_codes(diff.diff());

        // Diff only shows lines 3-8 (context), so width should be 1 (for single
        // digit numbers) NOT 4 (which would be needed for line 1000)
        assert!(clean_diff.contains("3 3 | line 3"));
        assert!(clean_diff.contains("5   |-line 5"));
        assert_eq!(diff.lines_added(), 1);
        assert_eq!(diff.lines_removed(), 1);
        assert_snapshot!(clean_diff);
    }
}
