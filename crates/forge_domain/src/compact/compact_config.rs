use derive_setters::Setters;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::{Context, ModelId, Role};

/// Strategy for generating summaries during compaction.
#[derive(Default, Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SummarizationStrategy {
    /// Pure structural extraction - extracts tool calls, file paths, and
    /// commands into a structured summary. Fast, deterministic, no API
    /// cost.
    #[default]
    Extract,

    /// LLM-based semantic summarization - uses an LLM to generate a coherent
    /// summary capturing decisions, rationale, and context. Higher quality
    /// but requires API call.
    Llm,

    /// Hybrid approach - first extracts structured data, then uses LLM to
    /// refine and enrich the summary with semantic understanding.
    Hybrid,
}

impl SummarizationStrategy {
    /// Returns true if this strategy requires LLM summarization
    pub fn requires_llm(&self) -> bool {
        matches!(self, Self::Llm | Self::Hybrid)
    }
}

/// Default timeout for LLM summarization (3 seconds)
fn default_summary_timeout() -> u64 {
    3
}

/// Configuration for automatic context compaction
#[derive(Debug, Clone, Serialize, Deserialize, Setters, JsonSchema, PartialEq)]
#[setters(strip_option, into)]
pub struct Compact {
    /// Number of most recent messages to preserve during compaction.
    /// These messages won't be considered for summarization. Works alongside
    /// eviction_window - the more conservative limit (fewer messages to
    /// compact) takes precedence.
    #[serde(default)]
    pub retention_window: usize,

    /// Maximum percentage of the context that can be summarized during
    /// compaction. Valid values are between 0.0 and 1.0, where 0.0 means no
    /// compaction and 1.0 allows summarizing all messages. Works alongside
    /// retention_window - the more conservative limit (fewer messages to
    /// compact) takes precedence.
    #[serde(default, deserialize_with = "deserialize_percentage")]
    pub eviction_window: f64,

    /// Maximum number of tokens to keep after compaction
    pub max_tokens: Option<usize>,

    /// Maximum number of tokens before triggering compaction. This acts as an
    /// absolute cap and is combined with
    /// `token_threshold_percentage` by taking the lower value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_threshold: Option<usize>,

    /// Maximum percentage of the model context window used to derive the token
    /// threshold before triggering compaction. This is combined with
    /// `token_threshold` by taking the lower value.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_percentage"
    )]
    pub token_threshold_percentage: Option<f64>,

    /// Maximum number of conversation turns before triggering compaction
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_threshold: Option<usize>,

    /// Maximum number of messages before triggering compaction
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_threshold: Option<usize>,

    /// Model ID to use for compaction, useful when compacting with a
    /// cheaper/faster model. If not specified, the root level model will be
    /// used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelId>,
    /// Whether to trigger compaction when the last message is from a user
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_turn_end: Option<bool>,

    /// Strategy for generating summaries during compaction.
    /// - `extract`: Pure structural extraction (default, fast, no API cost)
    /// - `llm`: Full LLM summarization (higher quality, requires API)
    /// - `hybrid`: Extract + LLM refinement (balanced)
    #[serde(default)]
    pub summarization_strategy: SummarizationStrategy,

    /// Model ID to use for LLM-based summarization. If not specified,
    /// falls back to `model` or the root level model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_model: Option<ModelId>,

    /// Maximum tokens in generated summary. Helps control output size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_max_tokens: Option<usize>,

    /// Timeout for LLM summarization in seconds. If exceeded, falls back
    /// to structural extraction.
    #[serde(default = "default_summary_timeout")]
    pub summary_timeout_secs: u64,

    /// Enable pre-compaction filtering to remove noise before summarization.
    /// Removes short tool results, debug output, and duplicate operations.
    #[serde(default)]
    pub enable_prefilter: bool,

    /// Enable adaptive eviction window that adjusts based on context ratio.
    /// More aggressive eviction when approaching token threshold.
    #[serde(default)]
    pub enable_adaptive_eviction: bool,

    /// Enable importance-based message preservation during eviction.
    /// High-importance messages (tool calls, errors, decisions) are protected.
    #[serde(default)]
    pub enable_importance_scoring: bool,

    // --- heliosLite fork: programmatic/semantic/AI-based compression ---
    /// Compression level for programmatic/semantic/AI strategies.
    /// 0 = off, 1 = programmatic only, 2 = + semantic, 3 = + AI-driven.
    #[serde(default)]
    pub context_compression_level: u32,

    /// Minimum importance score (0.0–1.0) for AI-driven pruning.
    /// Messages below this threshold are candidates for removal.
    #[serde(default)]
    pub min_importance_threshold: f64,

    /// Maximum number of messages to prune per compaction cycle.
    #[serde(default)]
    pub prune_threshold: usize,

    /// Enable semantic compression (embedding/cluster-based).
    #[serde(default)]
    pub enable_semantic_compression: bool,

    /// Enable structural deduplication (importance pruning).
    #[serde(default)]
    pub enable_structural_dedup: bool,

    /// Compression strategy identifier ("programmatic", "semantic", "ai",
    /// "all").
    #[serde(default)]
    pub compression_strategy: String,

    /// Prune strategy identifier ("importance", "position", "all").
    #[serde(default)]
    pub prune_strategy: String,
}

impl Compact {
    /// Applies a higher-precedence compaction configuration.
    ///
    /// Scalar fields always overwrite this configuration. Optional fields
    /// overwrite only when `other` provides a value.
    ///
    /// # Arguments
    ///
    /// * `other` - The higher-precedence compaction configuration.
    pub fn merge_from(&mut self, other: Self) {
        self.retention_window = other.retention_window;
        self.eviction_window = other.eviction_window;
        if other.max_tokens.is_some() {
            self.max_tokens = other.max_tokens;
        }
        if other.token_threshold.is_some() {
            self.token_threshold = other.token_threshold;
        }
        if other.token_threshold_percentage.is_some() {
            self.token_threshold_percentage = other.token_threshold_percentage;
        }
        if other.turn_threshold.is_some() {
            self.turn_threshold = other.turn_threshold;
        }
        if other.message_threshold.is_some() {
            self.message_threshold = other.message_threshold;
        }
        if other.model.is_some() {
            self.model = other.model;
        }
        if other.on_turn_end.is_some() {
            self.on_turn_end = other.on_turn_end;
        }
        self.summarization_strategy = other.summarization_strategy;
        if other.summary_model.is_some() {
            self.summary_model = other.summary_model;
        }
        if other.summary_max_tokens.is_some() {
            self.summary_max_tokens = other.summary_max_tokens;
        }
        self.summary_timeout_secs = other.summary_timeout_secs;
        self.enable_prefilter = other.enable_prefilter;
        self.enable_adaptive_eviction = other.enable_adaptive_eviction;
        self.enable_importance_scoring = other.enable_importance_scoring;
        self.context_compression_level = other.context_compression_level;
        self.min_importance_threshold = other.min_importance_threshold;
        self.prune_threshold = other.prune_threshold;
        self.enable_semantic_compression = other.enable_semantic_compression;
        self.enable_structural_dedup = other.enable_structural_dedup;
        self.compression_strategy = other.compression_strategy;
        self.prune_strategy = other.prune_strategy;
    }

    /// Applies only explicitly configured non-default scalar values from an
    /// agent overlay, preserving workflow values for a freshly constructed
    /// agent whose compact configuration is the default value.
    pub fn merge_non_default_from(&mut self, other: Self) {
        let defaults = Self::default();
        if other.retention_window != defaults.retention_window {
            self.retention_window = other.retention_window;
        }
        if other.eviction_window != defaults.eviction_window {
            self.eviction_window = other.eviction_window;
        }
        if other.summarization_strategy != defaults.summarization_strategy {
            self.summarization_strategy = other.summarization_strategy;
        }
        if other.summary_timeout_secs != defaults.summary_timeout_secs {
            self.summary_timeout_secs = other.summary_timeout_secs;
        }
        if other.enable_prefilter != defaults.enable_prefilter {
            self.enable_prefilter = other.enable_prefilter;
        }
        if other.enable_adaptive_eviction != defaults.enable_adaptive_eviction {
            self.enable_adaptive_eviction = other.enable_adaptive_eviction;
        }
        if other.enable_importance_scoring != defaults.enable_importance_scoring {
            self.enable_importance_scoring = other.enable_importance_scoring;
        }
        if other.context_compression_level != defaults.context_compression_level {
            self.context_compression_level = other.context_compression_level;
        }
        if other.min_importance_threshold != defaults.min_importance_threshold {
            self.min_importance_threshold = other.min_importance_threshold;
        }
        if other.prune_threshold != defaults.prune_threshold {
            self.prune_threshold = other.prune_threshold;
        }
        if other.enable_semantic_compression != defaults.enable_semantic_compression {
            self.enable_semantic_compression = other.enable_semantic_compression;
        }
        if other.enable_structural_dedup != defaults.enable_structural_dedup {
            self.enable_structural_dedup = other.enable_structural_dedup;
        }
        if other.compression_strategy != defaults.compression_strategy {
            self.compression_strategy = other.compression_strategy;
        }
        if other.prune_strategy != defaults.prune_strategy {
            self.prune_strategy = other.prune_strategy;
        }
        if other.max_tokens.is_some() {
            self.max_tokens = other.max_tokens;
        }
        if other.token_threshold.is_some() {
            self.token_threshold = other.token_threshold;
        }
        if other.token_threshold_percentage.is_some() {
            self.token_threshold_percentage = other.token_threshold_percentage;
        }
        if other.turn_threshold.is_some() {
            self.turn_threshold = other.turn_threshold;
        }
        if other.message_threshold.is_some() {
            self.message_threshold = other.message_threshold;
        }
        if other.model.is_some() {
            self.model = other.model;
        }
        if other.on_turn_end.is_some() {
            self.on_turn_end = other.on_turn_end;
        }
        if other.summary_model.is_some() {
            self.summary_model = other.summary_model;
        }
        if other.summary_max_tokens.is_some() {
            self.summary_max_tokens = other.summary_max_tokens;
        }
    }
}

fn deserialize_percentage<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    let value = f64::deserialize(deserializer)?;
    if !(0.0..=1.0).contains(&value) {
        return Err(Error::custom(format!(
            "percentage must be between 0.0 and 1.0, got {value}"
        )));
    }
    Ok(value)
}

fn deserialize_optional_percentage<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    let value = Option::<f64>::deserialize(deserializer)?;
    if let Some(value) = value
        && !(0.0..=1.0).contains(&value)
    {
        return Err(Error::custom(format!(
            "percentage must be between 0.0 and 1.0, got {value}"
        )));
    }
    Ok(value)
}

impl Default for Compact {
    fn default() -> Self {
        Self::new()
    }
}

impl Compact {
    /// Creates a new compaction configuration with the specified maximum token
    /// limit
    pub fn new() -> Self {
        Self {
            max_tokens: None,
            token_threshold: None,
            token_threshold_percentage: None,
            turn_threshold: None,
            message_threshold: None,
            model: None,
            eviction_window: 0.2, // Default to 20% compaction
            retention_window: 0,
            on_turn_end: None,
            summarization_strategy: SummarizationStrategy::default(),
            summary_model: None,
            summary_max_tokens: None,
            summary_timeout_secs: default_summary_timeout(),
            enable_prefilter: false,
            enable_adaptive_eviction: false,
            enable_importance_scoring: false,
            context_compression_level: 0,
            min_importance_threshold: 0.15,
            prune_threshold: 3,
            enable_semantic_compression: false,
            enable_structural_dedup: false,
            compression_strategy: String::new(),
            prune_strategy: String::new(),
        }
    }

    /// Determines if compaction should be triggered based on the current
    /// context
    pub fn should_compact(&self, context: &Context, token_count: usize) -> bool {
        self.should_compact_due_to_tokens(token_count)
            || self.should_compact_due_to_turns(context)
            || self.should_compact_due_to_messages(context)
            || self.should_compact_on_turn_end(context)
    }

    /// Checks if compaction should be triggered due to token count exceeding
    /// threshold
    fn should_compact_due_to_tokens(&self, token_count: usize) -> bool {
        if let Some(token_threshold) = self.token_threshold {
            debug!(tokens = ?token_count, "Token count");
            // use provided prompt_tokens if available, otherwise estimate token count
            token_count >= token_threshold
        } else {
            false
        }
    }

    /// Checks if compaction should be triggered due to turn count exceeding
    /// threshold
    fn should_compact_due_to_turns(&self, context: &Context) -> bool {
        if let Some(turn_threshold) = self.turn_threshold {
            context
                .messages
                .iter()
                .filter(|message| message.has_role(Role::User))
                .count()
                >= turn_threshold
        } else {
            false
        }
    }

    /// Checks if compaction should be triggered due to message count exceeding
    /// threshold
    fn should_compact_due_to_messages(&self, context: &Context) -> bool {
        if let Some(message_threshold) = self.message_threshold {
            // Count messages directly from context
            let msg_count = context.messages.len();
            msg_count >= message_threshold
        } else {
            false
        }
    }

    /// Checks if compaction should be triggered when the last message is from a
    /// user
    fn should_compact_on_turn_end(&self, context: &Context) -> bool {
        if let Some(true) = self.on_turn_end {
            context
                .messages
                .last()
                .map(|message| message.has_role(Role::User))
                .unwrap_or(false)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::MessagePattern;

    /// Creates a Context from a condensed string pattern where:
    /// - 'u' = User message
    /// - 'a' = Assistant message
    /// - 's' = System message Example: ctx("uau") creates User -> Assistant ->
    ///   User messages
    fn ctx(pattern: &str) -> Context {
        MessagePattern::new(pattern).build()
    }

    #[test]
    fn test_merge_from_overwrites_scalar_policies_and_preserves_absent_options() {
        let mut fixture = Compact::new()
            .retention_window(1_usize)
            .eviction_window(0.1_f64)
            .max_tokens(10_usize)
            .token_threshold(11_usize)
            .token_threshold_percentage(0.12_f64)
            .turn_threshold(13_usize)
            .message_threshold(14_usize)
            .model(ModelId::new("base-model"))
            .on_turn_end(true)
            .summarization_strategy(SummarizationStrategy::Extract)
            .summary_model(ModelId::new("base-summary"))
            .summary_max_tokens(15_usize)
            .summary_timeout_secs(16_u64)
            .enable_prefilter(false)
            .enable_adaptive_eviction(false)
            .enable_importance_scoring(false)
            .context_compression_level(1_u32)
            .min_importance_threshold(0.17_f64)
            .prune_threshold(18_usize)
            .enable_semantic_compression(false)
            .enable_structural_dedup(false)
            .compression_strategy("base-compression")
            .prune_strategy("base-prune");
        let other = Compact::new()
            .retention_window(21_usize)
            .eviction_window(0.22_f64)
            .summarization_strategy(SummarizationStrategy::Hybrid)
            .summary_timeout_secs(23_u64)
            .enable_prefilter(true)
            .enable_adaptive_eviction(true)
            .enable_importance_scoring(true)
            .context_compression_level(2_u32)
            .min_importance_threshold(0.24_f64)
            .prune_threshold(25_usize)
            .enable_semantic_compression(true)
            .enable_structural_dedup(true)
            .compression_strategy("other-compression")
            .prune_strategy("other-prune");
        fixture.merge_from(other);
        let expected = Compact::new()
            .retention_window(21_usize)
            .eviction_window(0.22_f64)
            .max_tokens(10_usize)
            .token_threshold(11_usize)
            .token_threshold_percentage(0.12_f64)
            .turn_threshold(13_usize)
            .message_threshold(14_usize)
            .model(ModelId::new("base-model"))
            .on_turn_end(true)
            .summarization_strategy(SummarizationStrategy::Hybrid)
            .summary_model(ModelId::new("base-summary"))
            .summary_max_tokens(15_usize)
            .summary_timeout_secs(23_u64)
            .enable_prefilter(true)
            .enable_adaptive_eviction(true)
            .enable_importance_scoring(true)
            .context_compression_level(2_u32)
            .min_importance_threshold(0.24_f64)
            .prune_threshold(25_usize)
            .enable_semantic_compression(true)
            .enable_structural_dedup(true)
            .compression_strategy("other-compression")
            .prune_strategy("other-prune");
        assert_eq!(fixture, expected);
    }

    #[test]
    fn test_merge_from_overwrites_all_present_option_policies() {
        let mut fixture = Compact::new()
            .max_tokens(10_usize)
            .token_threshold(11_usize)
            .token_threshold_percentage(0.12_f64)
            .turn_threshold(13_usize)
            .message_threshold(14_usize)
            .model(ModelId::new("base-model"))
            .on_turn_end(false)
            .summary_model(ModelId::new("base-summary"))
            .summary_max_tokens(15_usize);
        let other = Compact::new()
            .max_tokens(20_usize)
            .token_threshold(21_usize)
            .token_threshold_percentage(0.22_f64)
            .turn_threshold(23_usize)
            .message_threshold(24_usize)
            .model(ModelId::new("other-model"))
            .on_turn_end(true)
            .summary_model(ModelId::new("other-summary"))
            .summary_max_tokens(25_usize);
        fixture.merge_from(other);
        assert_eq!(fixture.max_tokens, Some(20));
        assert_eq!(fixture.token_threshold, Some(21));
        assert_eq!(fixture.token_threshold_percentage, Some(0.22));
        assert_eq!(fixture.turn_threshold, Some(23));
        assert_eq!(fixture.message_threshold, Some(24));
        assert_eq!(fixture.model, Some(ModelId::new("other-model")));
        assert_eq!(fixture.on_turn_end, Some(true));
        assert_eq!(fixture.summary_model, Some(ModelId::new("other-summary")));
        assert_eq!(fixture.summary_max_tokens, Some(25));
    }

    #[test]
    fn test_should_compact_due_to_tokens_exceeds_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .token_threshold(100_usize);
        let actual = fixture.should_compact_due_to_tokens(150);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_tokens_under_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .token_threshold(100_usize);
        let actual = fixture.should_compact_due_to_tokens(50);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_tokens_equals_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .token_threshold(100_usize);
        let actual = fixture.should_compact_due_to_tokens(100);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_tokens_no_threshold() {
        let fixture = Compact::new().model(ModelId::new("test-model"));
        let actual = fixture.should_compact_due_to_tokens(1000);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_turns_exceeds_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .turn_threshold(2_usize);
        let context = ctx("uauau");

        let actual = fixture.should_compact_due_to_turns(&context);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_turns_under_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .turn_threshold(3_usize);
        let context = ctx("ua");
        let actual = fixture.should_compact_due_to_turns(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_turns_equals_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .turn_threshold(2_usize);
        let context = ctx("uau");
        let actual = fixture.should_compact_due_to_turns(&context);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_turns_no_threshold() {
        let fixture = Compact::new().model(ModelId::new("test-model"));
        let context = ctx("uuu");
        let actual = fixture.should_compact_due_to_turns(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_turns_ignores_non_user_messages() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .turn_threshold(2_usize);
        let context = ctx("uasa");
        let actual = fixture.should_compact_due_to_turns(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_messages_exceeds_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .message_threshold(3_usize);
        let context = ctx("uaua");
        let actual = fixture.should_compact_due_to_messages(&context);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_messages_under_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .message_threshold(5_usize);
        let context = ctx("ua");
        let actual = fixture.should_compact_due_to_messages(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_messages_equals_threshold() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .message_threshold(3_usize);
        let context = ctx("uau");
        let actual = fixture.should_compact_due_to_messages(&context);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_messages_no_threshold() {
        let fixture = Compact::new().model(ModelId::new("test-model"));
        let context = ctx("uauau");
        let actual = fixture.should_compact_due_to_messages(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_no_thresholds_set() {
        let fixture = Compact::new().model(ModelId::new("test-model"));
        let context = ctx("ua");
        let actual = fixture.should_compact(&context, 1000);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_token_threshold_triggers() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .token_threshold(100_usize);
        let context = ctx("u");
        let actual = fixture.should_compact(&context, 150);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_turn_threshold_triggers() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .turn_threshold(1_usize);
        let context = ctx("uau");
        let actual = fixture.should_compact(&context, 50);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_message_threshold_triggers() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .message_threshold(2_usize);
        let context = ctx("uau");
        let actual = fixture.should_compact(&context, 50);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_multiple_thresholds_any_triggers() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .token_threshold(200_usize)
            .turn_threshold(5_usize)
            .message_threshold(10_usize);
        let context = ctx("ua");
        let actual = fixture.should_compact(&context, 250); // Only token threshold exceeded
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_multiple_thresholds_none_trigger() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .token_threshold(200_usize)
            .turn_threshold(5_usize)
            .message_threshold(10_usize);
        let context = ctx("ua");
        let actual = fixture.should_compact(&context, 100); // All thresholds under limit
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_empty_context() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .message_threshold(1_usize);
        let context = ctx("");
        let actual = fixture.should_compact(&context, 0);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_last_user_message_enabled_user_last() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .on_turn_end(true);
        let context = ctx("au");
        let actual = fixture.should_compact_on_turn_end(&context);
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_last_user_message_enabled_assistant_last() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .on_turn_end(true);
        let context = ctx("ua");
        let actual = fixture.should_compact_on_turn_end(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_last_user_message_enabled_system_last() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .on_turn_end(true);
        let context = ctx("us");
        let actual = fixture.should_compact_on_turn_end(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_last_user_message_disabled() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .on_turn_end(false);
        let context = ctx("au");
        let actual = fixture.should_compact_on_turn_end(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_last_user_message_not_configured() {
        let fixture = Compact::new().model(ModelId::new("test-model")); // No configuration set
        let context = ctx("au");
        let actual = fixture.should_compact_on_turn_end(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_due_to_last_user_message_empty_context() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .on_turn_end(true);
        let context = ctx("");
        let actual = fixture.should_compact_on_turn_end(&context);
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_last_user_message_integration() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .on_turn_end(true);
        let context = ctx("au");
        let actual = fixture.should_compact(&context, 10); // Low token count, no other thresholds
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_last_user_message_integration_disabled() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .on_turn_end(false);
        let context = ctx("au");
        let actual = fixture.should_compact(&context, 10); // Low token count, no other thresholds
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_should_compact_multiple_conditions_with_last_user_message() {
        let fixture = Compact::new()
            .model(ModelId::new("test-model"))
            .token_threshold(200_usize)
            .on_turn_end(true);
        let context = ctx("au");
        let actual = fixture.should_compact(&context, 50); // Token threshold not met, but last message is user
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_compact_model_none_falls_back_to_agent_model() {
        // Fixture
        let compact = Compact::new()
            .token_threshold(1000_usize)
            .turn_threshold(5_usize);

        // Assert
        assert_eq!(compact.model, None);
        assert_eq!(compact.token_threshold, Some(1000_usize));
        assert_eq!(compact.turn_threshold, Some(5_usize));
    }
}
