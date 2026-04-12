use std::collections::HashMap;
use std::path::PathBuf;

use derive_setters::Setters;
use fake::Dummy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::reader::ConfigReader;
use crate::writer::ConfigWriter;
use crate::{
    AutoDumpFormat, Compact, Decimal, HttpConfig, ModelConfig, ReasoningConfig, RetryConfig, Update,
};

/// Wire protocol a provider uses for chat completions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Dummy)]
pub enum ProviderResponseType {
    OpenAI,
    OpenAIResponses,
    Anthropic,
    Bedrock,
    Google,
    OpenCode,
}

/// Category of a provider.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, Dummy)]
#[serde(rename_all = "snake_case")]
pub enum ProviderTypeEntry {
    /// LLM provider for chat completions.
    #[default]
    Llm,
    /// Context engine provider for code indexing and search.
    ContextEngine,
}

/// Authentication method supported by a provider.
///
/// Only the simple (non-OAuth) methods are available here; providers that
/// require OAuth device or authorization-code flows must be configured via the
/// file-based `provider.json` override instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Dummy)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthMethod {
    ApiKey,
    GoogleAdc,
}

/// A URL parameter variable for a provider, used to substitute template
/// variables in URL strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Dummy)]
#[serde(rename_all = "snake_case")]
pub struct ProviderUrlParam {
    /// The environment variable name used as the template variable key.
    pub name: String,
    /// Optional preset values for this parameter shown as suggestions in the
    /// UI.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

/// A single provider entry defined inline in `forge.toml`.
///
/// Inline providers are merged with the built-in provider list; entries with
/// the same `id` override the corresponding built-in entry field-by-field,
/// while entries with a new `id` are appended to the list.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Dummy)]
#[serde(rename_all = "snake_case")]
pub struct ProviderEntry {
    /// Unique provider identifier used in model paths (e.g. `"my_provider"`).
    pub id: String,
    /// Environment variable holding the API key for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_var: Option<String>,
    /// URL template for chat completions; may contain `{{VAR}}` placeholders
    /// that are substituted from the credential's url params.
    pub url: String,
    /// URL template for fetching the model list; may contain `{{VAR}}`
    /// placeholders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<String>,
    /// Wire protocol used by this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_type: Option<ProviderResponseType>,
    /// Environment variables whose values are substituted into `{{VAR}}`
    /// placeholders in the `url` and `models` templates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub url_param_vars: Vec<ProviderUrlParam>,
    /// Additional HTTP headers sent with every request to this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_headers: Option<HashMap<String, String>>,
    /// Provider category; defaults to `llm` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<ProviderTypeEntry>,
    /// Authentication methods supported by this provider; defaults to
    /// `["api_key"]` when omitted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auth_methods: Vec<ProviderAuthMethod>,
}

/// Top-level Forge configuration merged from all sources (defaults, file,
/// environment).
#[derive(Default, Debug, Setters, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Dummy)]
#[serde(rename_all = "snake_case")]
#[setters(strip_option)]
pub struct ForgeConfig {
    /// Retry settings applied at the system level to all IO operations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryConfig>,
    /// Maximum number of lines returned by a single file search operation.
    #[serde(default)]
    pub max_search_lines: usize,
    /// Maximum number of bytes returned by a single file search operation.
    #[serde(default)]
    pub max_search_result_bytes: usize,
    /// Maximum number of characters returned from a URL fetch.
    #[serde(default)]
    pub max_fetch_chars: usize,
    /// Maximum number of lines captured from the leading portion of shell
    /// command output.
    #[serde(default)]
    pub max_stdout_prefix_lines: usize,
    /// Maximum number of lines captured from the trailing portion of shell
    /// command output.
    #[serde(default)]
    pub max_stdout_suffix_lines: usize,
    /// Maximum number of characters per line in shell command output.
    #[serde(default)]
    pub max_stdout_line_chars: usize,
    /// Maximum number of characters per line when reading a file.
    #[serde(default)]
    pub max_line_chars: usize,
    /// Maximum number of lines read from a file in a single operation.
    #[serde(default)]
    pub max_read_lines: u64,
    /// Maximum number of files read in a single batch operation.
    #[serde(default)]
    pub max_file_read_batch_size: usize,
    /// HTTP client settings including proxy, TLS, and timeout configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<HttpConfig>,
    /// Maximum file size in bytes permitted for read operations.
    #[serde(default)]
    pub max_file_size_bytes: u64,
    /// Maximum image file size in bytes permitted for read operations.
    #[serde(default)]
    pub max_image_size_bytes: u64,
    /// Maximum time in seconds a single tool call may run before being
    /// cancelled.
    #[serde(default)]
    pub tool_timeout_secs: u64,
    /// Whether to automatically open HTML dump files in the browser after
    /// creation.
    #[serde(default)]
    pub auto_open_dump: bool,
    /// Directory where debug request files are written; disabled when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_requests: Option<PathBuf>,
    /// Path to the conversation history file; defaults to the global history
    /// location when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_history_path: Option<PathBuf>,
    /// Maximum number of conversations shown in the conversation list.
    #[serde(default)]
    pub max_conversations: usize,
    /// Maximum number of candidate results returned from the initial semantic
    /// search vector query.
    #[serde(default)]
    pub max_sem_search_results: usize,
    /// Number of top results retained after re-ranking in semantic search.
    #[serde(default)]
    pub sem_search_top_k: usize,
    /// Base URL of the Forge services API used for semantic search and
    /// indexing.
    #[serde(default)]
    #[dummy(expr = "\"https://api.forgecode.dev/api\".to_string()")]
    pub services_url: String,
    /// Maximum number of file extensions included in the agent system prompt.
    #[serde(default)]
    pub max_extensions: usize,
    /// Format used when automatically creating a session dump after task
    /// completion; disabled when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_dump: Option<AutoDumpFormat>,
    /// Maximum number of files read concurrently during batch operations.
    #[serde(default)]
    pub max_parallel_file_reads: usize,
    /// Time-to-live in seconds for the cached model API list.
    #[serde(default)]
    pub model_cache_ttl_secs: u64,
    /// Default model and provider configuration used when not overridden by
    /// individual agents.    
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ModelConfig>,
    /// Model and provider configuration used for commit message generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<ModelConfig>,
    /// Maximum number of recent commits included as context for commit message
    /// generation.
    #[serde(default)]
    pub max_commit_count: usize,
    /// Model and provider configuration used for shell command suggestion
    /// generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggest: Option<ModelConfig>,

    // --- Workflow fields ---
    /// Configuration for automatic Forge updates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates: Option<Update>,

    /// Output randomness for all agents; lower values are deterministic, higher
    /// values are creative (0.0–2.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Decimal>,

    /// Nucleus sampling threshold for all agents; limits token selection to the
    /// top cumulative probability mass (0.0–1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<Decimal>,

    /// Top-k vocabulary cutoff for all agents; restricts sampling to the k
    /// highest-probability tokens (1–1000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,

    /// Maximum tokens the model may generate per response for all agents
    /// (1–100,000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,

    /// Maximum tool failures per turn before the orchestrator forces
    /// completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_failure_per_turn: Option<usize>,

    /// Maximum number of requests that can be made in a single turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_requests_per_turn: Option<usize>,

    /// Context compaction settings applied to all agents; falls back to each
    /// agent's individual setting when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact: Option<Compact>,

    /// Whether restricted mode is active; when enabled, tool execution requires
    /// explicit permission grants.
    #[serde(default)]
    pub restricted: bool,

    /// Whether tool use is supported in the current environment; when false,
    /// all tool calls are disabled.
    #[serde(default)]
    pub tool_supported: bool,

    /// Reasoning configuration applied to all agents; controls effort level,
    /// token budget, and visibility of the model's thinking process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,

    /// Additional provider definitions merged with the built-in provider list.
    ///
    /// Entries with an `id` matching a built-in provider override its fields;
    /// entries with a new `id` are appended and become available for model
    /// selection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<ProviderEntry>,

    /// Currency symbol displayed in the shell rprompt next to the session cost
    /// (e.g. `"$"`, `"€"`, `"₹"`). Defaults to `"$"`.
    #[serde(default)]
    pub currency_symbol: String,

    /// Conversion rate applied to costs before display in the shell rprompt.
    /// The raw USD cost is multiplied by this value, allowing costs to be shown
    /// in a local currency. Defaults to `1.0` (no conversion).
    #[serde(default)]
    pub currency_conversion_rate: Decimal,

    /// Enables the pending todos hook that checks for incomplete todo items
    /// when a task ends and reminds the LLM about them.
    #[serde(default)]
    pub verify_todos: bool,

    /// Whether to include terminal context (recent shell commands, exit codes,
    /// and timestamps) in the user prompt. When enabled, the shell plugin
    /// exports the captured command history and the Rust service injects it
    /// into the rendered prompt via [`EventContext`].
    ///
    /// Defaults to `false` so that users explicitly opt in before their shell
    /// history is sent to the LLM. Shell history can contain passwords, API
    /// keys, and other sensitive data passed as command-line arguments.
    #[serde(default)]
    pub terminal_context: bool,

    /// Maximum number of recent terminal commands included in the terminal
    /// context injected into the user prompt. Commands are sorted by timestamp
    /// so that the most recent command appears last. When set to `0` (the
    /// default), all captured commands are included.
    #[serde(default)]
    pub max_terminal_commands: usize,
}

impl ForgeConfig {
    /// Reads and merges configuration from all sources, returning the resolved
    /// [`ForgeConfig`].
    ///
    /// # Errors
    ///
    /// Returns an error if the config path cannot be resolved, the file cannot
    /// be read, or deserialization fails.
    pub fn read() -> crate::Result<ForgeConfig> {
        ConfigReader::default()
            .read_legacy()
            .read_defaults()
            .read_global()
            .read_env()
            .build()
    }

    /// Writes the configuration to the user config file.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration cannot be serialized or written to
    /// disk.
    pub fn write(&self) -> crate::Result<()> {
        let path = ConfigReader::config_path();
        ConfigWriter::new(self.clone()).write(&path)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::reader::ConfigReader;

    #[test]
    fn test_f32_temperature_round_trip() {
        let fixture = ForgeConfig { temperature: Some(Decimal(0.1)), ..Default::default() };

        let toml = toml_edit::ser::to_string_pretty(&fixture).unwrap();

        assert!(
            toml.contains("temperature = 0.1\n"),
            "expected `temperature = 0.1` in TOML output, got:\n{toml}"
        );
    }

    #[test]
    fn test_f32_top_p_round_trip() {
        let fixture = ForgeConfig { top_p: Some(Decimal(0.9)), ..Default::default() };

        let toml = toml_edit::ser::to_string_pretty(&fixture).unwrap();

        assert!(
            toml.contains("top_p = 0.9\n"),
            "expected `top_p = 0.9` in TOML output, got:\n{toml}"
        );
    }

    #[test]
    fn test_f32_temperature_deserialize_round_trip() {
        let fixture = ForgeConfig { temperature: Some(Decimal(0.1)), ..Default::default() };

        let toml = toml_edit::ser::to_string_pretty(&fixture).unwrap();

        let actual = ConfigReader::default().read_toml(&toml).build().unwrap();

        assert_eq!(actual.temperature, fixture.temperature);
    }
}
