use derive_setters::Setters;
use fake::Dummy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Controls the verbosity of forge's tool output formatting.
///
/// The output mode affects how tool results are rendered in the chat UI:
/// - `Concise`: Minimal output, just the essential information (default for
///   most users).
/// - `Compact`: Same as concise but with extra whitespace trimming and
///   aggressive line folding for terminal-friendly display.
/// - `Verbose`: Full output including all metadata, reasoning traces, and
///   intermediate computation steps. Useful for debugging.
#[derive(
    Default, Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Dummy,
)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    /// Minimal output (default).
    #[default]
    Concise,
    /// Extra whitespace-trimmed variant of concise for terminal display.
    Compact,
    /// Full output with all metadata and intermediate steps.
    Verbose,
}

impl OutputMode {
    /// Returns true if the mode prefers minimal line breaks and whitespace
    /// trimming.
    pub fn is_compact(&self) -> bool {
        matches!(self, Self::Compact | Self::Concise)
    }

    /// Returns true if the mode includes detailed metadata such as reasoning
    /// traces, intermediate computations, and diagnostic breadcrumbs.
    pub fn is_verbose(&self) -> bool {
        matches!(self, Self::Verbose)
    }

    /// Returns a short human-readable label for this mode, suitable for
    /// status messages and TUI feedback.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Concise => "concise",
            Self::Compact => "compact",
            Self::Verbose => "verbose",
        }
    }
}

/// User-facing configuration for tool output rendering.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema, Setters, PartialEq, Dummy)]
#[setters(strip_option, into)]
pub struct OutputSettings {
    /// Verbosity level applied to tool output rendering.
    #[serde(default)]
    pub mode: OutputMode,

    /// Whether to include a trailing newline after tool output blocks.
    /// Defaults to `true`. Disable to suppress extra blank lines in agents
    /// that add their own formatting.
    #[serde(default = "default_true")]
    pub trailing_newline: bool,
}

fn default_true() -> bool {
    true
}

impl OutputSettings {
    /// Apply the configured mode to a string slice, returning the rendered
    /// text. In `Compact` mode leading/trailing whitespace is trimmed from
    /// each line and consecutive blank lines are collapsed. Other modes pass
    /// the input through unchanged.
    pub fn render(&self, input: &str) -> String {
        if !self.mode.is_compact() {
            return input.to_string();
        }
        let mut out = String::with_capacity(input.len());
        let mut emitted_any = false;
        for line in input.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                // Skip blank lines entirely; `compact` mode collapses them.
                continue;
            }
            if emitted_any {
                out.push('\n');
            }
            out.push_str(trimmed);
            emitted_any = true;
        }
        if self.trailing_newline && emitted_any && !out.ends_with('\n') {
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_output_mode_default_is_concise() {
        assert_eq!(OutputMode::default(), OutputMode::Concise);
    }

    #[test]
    fn test_output_mode_is_compact() {
        assert!(OutputMode::Concise.is_compact());
        assert!(OutputMode::Compact.is_compact());
        assert!(!OutputMode::Verbose.is_compact());
    }

    #[test]
    fn test_output_mode_is_verbose() {
        assert!(OutputMode::Verbose.is_verbose());
        assert!(!OutputMode::Concise.is_verbose());
        assert!(!OutputMode::Compact.is_verbose());
    }

    #[test]
    fn test_output_settings_verbose_render_is_passthrough() {
        let s = OutputSettings {
            mode: OutputMode::Verbose,
            trailing_newline: true,
        };
        let input = "  hello  \n\n  world  \n";
        assert_eq!(s.render(input), input);
    }

    #[test]
    fn test_output_settings_compact_trims_lines() {
        let s = OutputSettings {
            mode: OutputMode::Compact,
            trailing_newline: true,
        };
        let input = "  hello  \n  world  \n";
        assert_eq!(s.render(input), "hello\nworld\n");
    }

    #[test]
    fn test_output_settings_compact_collapses_blank_lines() {
        let s = OutputSettings {
            mode: OutputMode::Compact,
            trailing_newline: true,
        };
        let input = "a\n\n\n\nb\n";
        assert_eq!(s.render(input), "a\nb\n");
    }

    #[test]
    fn test_output_settings_concise_does_not_add_trailing_newline_when_disabled() {
        let s = OutputSettings {
            mode: OutputMode::Concise,
            trailing_newline: false,
        };
        let input = "hello";
        assert_eq!(s.render(input), "hello");
    }

    #[test]
    fn test_output_settings_round_trip() {
        let fixture = OutputSettings {
            mode: OutputMode::Verbose,
            trailing_newline: false,
        };

        let toml = toml_edit::ser::to_string_pretty(&fixture).unwrap();

        assert!(toml.contains("mode = \"verbose\""));
        assert!(toml.contains("trailing_newline = false"));
    }
}
