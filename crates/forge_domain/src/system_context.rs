use derive_setters::Setters;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{Agent, Environment, File, Model, Skill};

/// Statistics for a file extension
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtensionStat {
    /// File extension (e.g., "rs", "md", "toml")
    pub extension: String,
    /// Number of files with this extension
    pub count: usize,
    /// Percentage of total files (formatted to 2 decimal places, e.g., "51.42")
    pub percentage: String,
}

impl ExtensionStat {
    /// Creates a new [`ExtensionStat`] with the given extension, count, and
    /// percentage.
    pub fn new(extension: impl Into<String>, count: usize, percentage: impl Into<String>) -> Self {
        Self {
            extension: extension.into(),
            count,
            percentage: percentage.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extension {
    pub extension_stats: Vec<ExtensionStat>,
    pub max_extensions: usize,
    pub git_tracked_files: usize,
    pub total_extensions: usize,
    /// Percentage of files covered by remaining (non-displayed) extensions
    pub remaining_percentage: String,
}

impl Extension {
    /// Creates a new [`Extension`] summary.
    pub fn new(
        extension_stats: Vec<ExtensionStat>,
        max_extensions: usize,
        git_tracked_files: usize,
        total_extensions: usize,
        remaining_percentage: impl Into<String>,
    ) -> Self {
        Self {
            extension_stats,
            max_extensions,
            git_tracked_files,
            total_extensions,
            remaining_percentage: remaining_percentage.into(),
        }
    }
}

/// Configuration values required by tool description templates.
///
/// Populated from [`ForgeConfig`] by the application layer and injected into
/// [`SystemContext`] so that Handlebars templates can reference values such as
/// `{{config.maxReadSize}}` without coupling `SystemContext` to `ForgeConfig`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TemplateConfig {
    /// Maximum number of lines returned by a single file read (maps to
    /// `ForgeConfig::max_read_lines`).
    pub max_read_size: usize,
    /// Maximum characters per line before truncation (maps to
    /// `ForgeConfig::max_line_chars`).
    pub max_line_length: usize,
    /// Maximum image size in bytes accepted by the read tool (maps to
    /// `ForgeConfig::max_image_size_bytes`).
    pub max_image_size: usize,
    /// Maximum prefix lines kept when truncating shell stdout (maps to
    /// `ForgeConfig::max_stdout_prefix_lines`).
    pub stdout_max_prefix_length: usize,
    /// Maximum suffix lines kept when truncating shell stdout (maps to
    /// `ForgeConfig::max_stdout_suffix_lines`).
    pub stdout_max_suffix_length: usize,
    /// Maximum characters per line in shell stdout before truncation (maps to
    /// `ForgeConfig::max_stdout_line_chars`).
    pub stdout_max_line_length: usize,
}

#[derive(Debug, Setters, Clone, PartialEq, Serialize, Deserialize)]
#[setters(strip_option)]
#[derive(Default)]
pub struct SystemContext {
    // Environment information to be included in the system context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<Environment>,

    // Information about available tools that can be used by the agent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_information: Option<String>,

    /// Indicates whether the agent supports tools.
    /// This value is populated directly from the Agent configuration.
    #[serde(default)]
    pub tool_supported: bool,

    // List of files and directories that are relevant for the agent context
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<File>,

    #[serde(skip_serializing_if = "String::is_empty")]
    pub custom_rules: String,

    /// Indicates whether the agent supports parallel tool calls.
    #[serde(default)]
    pub supports_parallel_tool_calls: bool,

    /// List of available skills
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<Skill>,

    /// Currently selected model with capabilities
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<Model>,

    /// Map of tool names for template rendering.
    /// Keys are tool identifiers (e.g., "read", "write"), values are display
    /// names. Accessed in templates as {{tool_names.read}},
    /// {{tool_names.write}}, etc.
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub tool_names: Map<String, Value>,

    /// File extension statistics sorted by count (descending), limited to the
    /// top `limit` extensions as defined in the `Extension` struct.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Extension>,

    /// List of available agents for task delegation
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<Agent>,

    /// Template configuration for tool descriptions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<TemplateConfig>,
}
