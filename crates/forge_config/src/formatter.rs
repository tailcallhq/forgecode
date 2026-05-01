//! Tool output formatter for compact/concise display.

use super::output::{OutputSettings, Verbosity};

/// Formatter for tool call output based on OutputSettings.
#[derive(Debug, Clone)]
pub struct ToolOutputFormatter {
    settings: OutputSettings,
}

impl ToolOutputFormatter {
    /// Create a new formatter with the given settings.
    pub fn new(settings: OutputSettings) -> Self {
        Self { settings }
    }

    /// Create from ForgeConfig's output settings (with defaults if none).
    pub fn from_config(output_config: Option<&OutputSettings>) -> Self {
        Self { settings: output_config.cloned().unwrap_or_default() }
    }

    /// Format a tool result for display based on current verbosity.
    pub fn format_tool_result(&self, tool_name: &str, result: &str, is_error: bool) -> String {
        let settings = &self.settings;

        // Compact mode: single line
        if settings.is_compact() {
            return self.format_compact(tool_name, result, is_error);
        }

        // Concise mode: brief summary
        if matches!(settings.effective_verbosity(), Verbosity::Concise) {
            return self.format_concise(tool_name, result, is_error);
        }

        // Normal/Verbose: full output (or error details)
        if is_error || settings.expand_errors {
            return self.format_full(tool_name, result);
        }

        // Normal: collapsed success
        if settings.collapse_success {
            return self.format_collapsed(tool_name, result);
        }

        self.format_full(tool_name, result)
    }

    /// Compact: `[tool_name] result_summary`
    fn format_compact(&self, tool_name: &str, result: &str, is_error: bool) -> String {
        let short_name = self.settings.format_tool_name(tool_name);
        let summary = self.settings.format_result_summary(result);
        if is_error {
            format!("{short_name} ERR: {summary}")
        } else {
            format!("{short_name} OK")
        }
    }

    /// Concise: brief summary without full output.
    fn format_concise(&self, tool_name: &str, result: &str, is_error: bool) -> String {
        let short_name = self.settings.format_tool_name(tool_name);
        if is_error {
            let summary = self.settings.format_result_summary(result);
            format!("{short_name}: ERR - {summary}")
        } else {
            let lines = result.lines().count();
            format!("{short_name}: {lines} lines")
        }
    }

    /// Full output with truncation.
    fn format_full(&self, tool_name: &str, result: &str) -> String {
        let max_lines = self.settings.max_tool_lines;
        let truncate = self.settings.truncate_chars;

        let mut output = format!("{tool_name}:\n");

        for (i, line) in result.lines().enumerate() {
            if max_lines > 0 && i >= max_lines {
                output.push_str("... (truncated)\n");
                break;
            }

            if line.len() > truncate {
                output.push_str(&format!("{}\n", &line[..truncate - 3]));
            } else {
                output.push_str(&format!("{line}\n"));
            }
        }

        output
    }

    /// Collapsed success: just tool name + status.
    fn format_collapsed(&self, tool_name: &str, _result: &str) -> String {
        let short_name = self.settings.format_tool_name(tool_name);
        format!("{short_name} ✓")
    }

    /// Get the current verbosity level.
    pub fn verbosity(&self) -> Verbosity {
        self.settings.effective_verbosity()
    }

    /// Get a mutable reference to settings (for runtime changes).
    pub fn settings_mut(&mut self) -> &mut OutputSettings {
        &mut self.settings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compact_mode() {
        let settings = OutputSettings { compact: true, ..Default::default() };
        let formatter = ToolOutputFormatter::new(settings);

        let result = formatter.format_tool_result(
            "Read::read_file",
            "file contents here",
            false,
        );

        assert!(result.contains("read_file"));
        assert!(result.contains("OK"));
        assert!(result.len() < 100); // Should be short
    }

    #[test]
    fn test_error_expanded() {
        let settings = OutputSettings { expand_errors: true, ..Default::default() };
        let formatter = ToolOutputFormatter::new(settings);

        let result = formatter.format_tool_result("shell", "error: not found", true);

        assert!(result.contains("shell"));
        assert!(result.contains("error: not found"));
    }
}
