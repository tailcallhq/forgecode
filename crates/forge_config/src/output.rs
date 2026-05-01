//! Output formatting settings for Forge display.

use derive_setters::Setters;
use fake::Dummy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Output verbosity level for tool display.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Dummy)]
#[serde(rename_all = "snake_case")]
pub enum Verbosity {
    /// Ultra compact: 1 line per tool, no details
    Compact,
    /// Brief summaries, still condensed
    Concise,
    /// Standard output (default)
    #[default]
    Normal,
    /// Full details including all tool output
    Verbose,
}

/// Output formatting settings controlling how tools and responses are displayed.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Dummy, Setters)]
#[serde(rename_all = "snake_case")]
#[setters(strip_option)]
pub struct OutputSettings {
    /// Output verbosity level.
    #[serde(default)]
    pub verbosity: Verbosity,

    /// Compact mode: 1 line per tool output (implies verbosity: compact).
    /// Takes precedence over verbosity when true.
    #[serde(default)]
    pub compact: bool,

    /// Show tool headers/invocations.
    #[serde(default = "default_true")]
    pub show_tool_headers: bool,

    /// Maximum lines per tool output (0 = unlimited).
    #[serde(default)]
    pub max_tool_lines: usize,

    /// Truncate tool output beyond N characters per line.
    #[serde(default = "default_200")]
    pub truncate_chars: usize,

    /// Show tool execution time.
    #[serde(default = "default_true")]
    pub show_timing: bool,

    /// Show token counts for tool results.
    #[serde(default = "default_false")]
    pub show_tokens: bool,

    /// Collapse successful tool calls to single line.
    #[serde(default = "default_true")]
    pub collapse_success: bool,

    /// Expand failed tool calls to show errors.
    #[serde(default = "default_true")]
    pub expand_errors: bool,
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn default_200() -> usize {
    200
}

impl OutputSettings {
    /// Returns true if compact mode should be used.
    pub fn is_compact(&self) -> bool {
        self.compact || matches!(self.verbosity, Verbosity::Compact)
    }

    /// Returns the effective verbosity (compact overrides).
    pub fn effective_verbosity(&self) -> Verbosity {
        if self.compact {
            Verbosity::Compact
        } else {
            self.verbosity
        }
    }

    /// Format a tool name for compact display.
    pub fn format_tool_name(&self, tool_name: &str) -> String {
        if self.is_compact() {
            // Just the tool name, no path
            tool_name
                .rsplit("::")
                .next()
                .unwrap_or(tool_name)
                .to_string()
        } else {
            tool_name.to_string()
        }
    }

    /// Format result summary for compact display.
    pub fn format_result_summary(&self, result: &str) -> String {
        let result = result.trim();
        if result.is_empty() {
            return String::new();
        }

        // For compact mode, take first line and truncate
        let first_line = result.lines().next().unwrap_or(result);

        if first_line.len() > self.truncate_chars {
            format!("{}...", &first_line[..self.truncate_chars - 3])
        } else {
            first_line.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_compact() {
        let settings = OutputSettings {
            verbosity: Verbosity::Normal,
            compact: true,
            ..Default::default()
        };
        assert!(settings.is_compact());

        let settings = OutputSettings {
            verbosity: Verbosity::Compact,
            compact: false,
            ..Default::default()
        };
        assert!(settings.is_compact());
    }

    #[test]
    fn test_format_result_summary() {
        let settings = OutputSettings {
            truncate_chars: 20,
            ..Default::default()
        };

        let result = settings.format_result_summary("hello world this is long");
        assert_eq!(result.len(), 20);
        assert!(result.ends_with("..."));
    }
}
