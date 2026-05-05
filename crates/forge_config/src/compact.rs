use std::time::Duration;

use derive_setters::Setters;
use fake::Dummy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Percentage;

/// Strategy for generating summaries during compaction.
#[derive(
    Default, Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Dummy,
)]
#[serde(rename_all = "snake_case")]
pub enum SummarizationStrategy {
    /// Pure structural extraction - extracts tool calls, file paths, and commands
    /// into a structured summary. Fast, deterministic, no API cost.
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

/// Frequency at which forge checks for updates
#[derive(Default, Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, fake::Dummy)]
#[serde(rename_all = "snake_case")]
pub enum UpdateFrequency {
    Daily,
    Weekly,
    Never,
    #[default]
    Always,
}

impl From<UpdateFrequency> for Duration {
    fn from(val: UpdateFrequency) -> Self {
        match val {
            UpdateFrequency::Daily => Duration::from_secs(60 * 60 * 24), // 1 day
            UpdateFrequency::Weekly => Duration::from_secs(60 * 60 * 24 * 7), // 1 week
            UpdateFrequency::Never => Duration::MAX,
            UpdateFrequency::Always => Duration::ZERO, // one time
        }
    }
}

impl SummarizationStrategy {
    /// Returns true if this strategy requires LLM summarization
    pub fn requires_llm(&self) -> bool {
        matches!(self, Self::Llm | Self::Hybrid)
    }

    /// Returns the effective timeout duration for this strategy
    pub fn timeout(&self, secs: u64) -> Duration {
        Duration::from_secs(secs)
    }
}

/// Default timeout for LLM summarization (3 seconds)
fn default_summary_timeout() -> u64 {
    3
}

/// Configuration for automatic forge updates
#[derive(
    Debug, Clone, Serialize, Deserialize, Default, JsonSchema, Setters, PartialEq, fake::Dummy,
)]
#[setters(strip_option, into)]
pub struct Update {
    /// How frequently forge checks for updates: daily, weekly, always, or never
    pub frequency: Option<UpdateFrequency>,
    /// Whether to automatically install updates without prompting
    pub auto_update: Option<bool>,
}

/// Configuration for automatic context compaction for all agents
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
    #[serde(default)]
    pub eviction_window: Percentage,

    /// Maximum number of tokens to keep after compaction
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,

    /// Maximum number of tokens before triggering compaction. This acts as an
    /// absolute cap and is combined with
    /// `token_threshold_percentage` by taking the lower value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_threshold: Option<usize>,

    /// Maximum percentage of the model context window used to derive the token
    /// threshold before triggering compaction. This is combined with
    /// `token_threshold` by taking the lower value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_threshold_percentage: Option<Percentage>,

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
    pub model: Option<String>,

    /// Strategy for generating summaries during compaction.
    /// - `extract`: Pure structural extraction (default, fast, no API cost)
    /// - `llm`: Full LLM summarization (higher quality, requires API)
    /// - `hybrid`: Extract + LLM refinement (balanced)
    #[serde(default)]
    pub summarization_strategy: SummarizationStrategy,

    /// Model ID to use for LLM-based summarization. If not specified,
    /// falls back to `model` or the root level model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_model: Option<String>,

    /// Maximum tokens in generated summary. Helps control output size.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[setters(skip)]
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

    /// Whether to trigger compaction when the last message is from a user
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_turn_end: Option<bool>,
}

impl Default for Compact {
    fn default() -> Self {
        Self::new()
    }
}

impl Compact {
    /// Creates a new compaction configuration with all optional fields unset
    pub fn new() -> Self {
        Self {
            max_tokens: None,
            token_threshold: None,
            token_threshold_percentage: None,
            turn_threshold: None,
            message_threshold: None,
            model: None,
            eviction_window: Percentage::new(0.2).unwrap(),
            retention_window: 0,
            on_turn_end: None,
            summarization_strategy: SummarizationStrategy::default(),
            summary_model: None,
            summary_max_tokens: None,
            summary_timeout_secs: default_summary_timeout(),
            enable_prefilter: false,
            enable_adaptive_eviction: false,
            enable_importance_scoring: false,
        }
    }
}

impl Dummy<fake::Faker> for Compact {
    fn dummy_with_rng<R: fake::RngExt + ?Sized>(_: &fake::Faker, rng: &mut R) -> Self {
        use fake::Fake;
        Self {
            retention_window: fake::Faker.fake_with_rng(rng),
            eviction_window: Percentage::from((0.0f64..=1.0f64).fake_with_rng::<f64, R>(rng)),
            max_tokens: fake::Faker.fake_with_rng(rng),
            token_threshold: fake::Faker.fake_with_rng(rng),
            token_threshold_percentage: fake::Faker.fake_with_rng(rng),
            turn_threshold: fake::Faker.fake_with_rng(rng),
            message_threshold: fake::Faker.fake_with_rng(rng),
            model: fake::Faker.fake_with_rng(rng),
            on_turn_end: fake::Faker.fake_with_rng(rng),
            summarization_strategy: fake::Faker.fake_with_rng(rng),
            summary_model: fake::Faker.fake_with_rng(rng),
            summary_max_tokens: fake::Faker.fake_with_rng(rng),
            summary_timeout_secs: 3,
            enable_prefilter: fake::Faker.fake_with_rng(rng),
            enable_adaptive_eviction: fake::Faker.fake_with_rng(rng),
            enable_importance_scoring: fake::Faker.fake_with_rng(rng),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::ForgeConfig;
    use crate::reader::ConfigReader;

    #[test]
    fn test_f64_eviction_window_round_trip() {
        let fixture = Compact {
            eviction_window: Percentage::new(0.2).unwrap(),
            ..Compact::new()
        };

        let toml = toml_edit::ser::to_string_pretty(&fixture).unwrap();

        assert!(
            toml.contains("eviction_window = 0.2\n"),
            "expected `eviction_window = 0.2` in TOML output, got:\n{toml}"
        );
    }

    #[test]
    fn test_f64_eviction_window_deserialize_round_trip() {
        let fixture = Compact {
            eviction_window: Percentage::new(0.2).unwrap(),
            ..Compact::new()
        };
        let config_fixture = ForgeConfig::default().compact(fixture.clone());

        let toml = toml_edit::ser::to_string_pretty(&config_fixture).unwrap();

        let actual = ConfigReader::default()
            .read_defaults()
            .read_toml(&toml)
            .build()
            .unwrap();
        let actual = actual.compact.expect("compact config should deserialize");

        assert_eq!(actual.eviction_window, fixture.eviction_window);
    }

    #[test]
    fn test_token_threshold_percentage_round_trip() {
        let fixture = Compact {
            token_threshold_percentage: Some(Percentage::new(0.7).unwrap()),
            ..Compact::new()
        };
        let config_fixture = ForgeConfig::default().compact(fixture.clone());

        let toml = toml_edit::ser::to_string_pretty(&config_fixture).unwrap();

        assert!(
            toml.contains("token_threshold_percentage = 0.7\n"),
            "expected `token_threshold_percentage = 0.7` in TOML output, got:\n{toml}"
        );

        let actual = ConfigReader::default()
            .read_defaults()
            .read_toml(&toml)
            .build()
            .unwrap();
        let actual = actual.compact.expect("compact config should deserialize");

        assert_eq!(
            actual.token_threshold_percentage,
            fixture.token_threshold_percentage
        );
    }

    #[test]
    fn test_token_threshold_percentage_rejects_out_of_range() {
        let toml = "[compact]\ntoken_threshold_percentage = 1.5\n";

        let result = ConfigReader::default()
            .read_defaults()
            .read_toml(toml)
            .build();

        assert!(
            result.is_err(),
            "expected error for token_threshold_percentage = 1.5, got: {:?}",
            result.ok()
        );
    }

    #[test]
    fn test_eviction_window_rejects_out_of_range() {
        let toml = "[compact]\neviction_window = 1.5\n";

        let result = ConfigReader::default()
            .read_defaults()
            .read_toml(toml)
            .build();

        assert!(
            result.is_err(),
            "expected error for eviction_window = 1.5, got: {:?}",
            result.ok()
        );
    }

    #[test]
    fn test_update_frequency_never_round_trip() {
        let fixture =
            ForgeConfig::default().updates(Update::default().frequency(UpdateFrequency::Never));

        let toml = toml_edit::ser::to_string_pretty(&fixture).unwrap();

        assert!(
            toml.contains("frequency = \"never\"\n"),
            "expected `frequency = \"never\"` in TOML output, got:\n{toml}"
        );

        let actual = ConfigReader::default()
            .read_defaults()
            .read_toml(&toml)
            .build()
            .unwrap();

        let expected = Some(
            Update::default()
                .frequency(UpdateFrequency::Never)
                .auto_update(true),
        );
        assert_eq!(actual.updates, expected);
    }

    #[test]
    fn test_summarization_strategy_default_is_extract() {
        assert_eq!(SummarizationStrategy::default(), SummarizationStrategy::Extract);
    }

    #[test]
    fn test_summarization_strategy_requires_llm() {
        assert!(!SummarizationStrategy::Extract.requires_llm());
        assert!(SummarizationStrategy::Llm.requires_llm());
        assert!(SummarizationStrategy::Hybrid.requires_llm());
    }

    #[test]
    fn test_summarization_strategy_timeout() {
        let strategy = SummarizationStrategy::Llm;
        assert_eq!(strategy.timeout(3), Duration::from_secs(3));
        assert_eq!(strategy.timeout(5), Duration::from_secs(5));
    }

    #[test]
    fn test_summarization_strategy_round_trip() {
        for strategy in [
            SummarizationStrategy::Extract,
            SummarizationStrategy::Llm,
            SummarizationStrategy::Hybrid,
        ] {
            let fixture = Compact::new().summarization_strategy(strategy);
            let config_fixture = ForgeConfig::default().compact(fixture.clone());

            let toml = toml_edit::ser::to_string_pretty(&config_fixture).unwrap();

            let actual = ConfigReader::default()
                .read_defaults()
                .read_toml(&toml)
                .build()
                .unwrap();
            let actual = actual.compact.expect("compact config should deserialize");

            assert_eq!(actual.summarization_strategy, strategy);
        }
    }

    #[test]
    fn test_compact_new_has_default_values() {
        let compact = Compact::new();
        assert_eq!(compact.summarization_strategy, SummarizationStrategy::Extract);
        assert_eq!(compact.summary_timeout_secs, 3);
        assert!(!compact.enable_prefilter);
        assert!(!compact.enable_adaptive_eviction);
        assert!(!compact.enable_importance_scoring);
        assert!(compact.summary_model.is_none());
        assert!(compact.summary_max_tokens.is_none());
    }

    #[test]
    fn test_compact_with_enhancements_round_trip() {
        let mut fixture = Compact::new();
        fixture.summarization_strategy = SummarizationStrategy::Hybrid;
        fixture.summary_model = Some("claude-3-5-haiku".to_string());
        fixture.summary_max_tokens = Some(4000);
        fixture.summary_timeout_secs = 5;
        fixture.enable_prefilter = true;
        fixture.enable_adaptive_eviction = true;
        fixture.enable_importance_scoring = true;

        let config_fixture = ForgeConfig::default().compact(fixture.clone());

        let toml = toml_edit::ser::to_string_pretty(&config_fixture).unwrap();

        let actual = ConfigReader::default()
            .read_defaults()
            .read_toml(&toml)
            .build()
            .unwrap();
        let actual = actual.compact.expect("compact config should deserialize");

        assert_eq!(actual.summarization_strategy, SummarizationStrategy::Hybrid);
        assert_eq!(actual.summary_model, Some("claude-3-5-haiku".to_string()));
        assert_eq!(actual.summary_max_tokens, Some(4000));
        assert_eq!(actual.summary_timeout_secs, 5);
        assert!(actual.enable_prefilter);
        assert!(actual.enable_adaptive_eviction);
        assert!(actual.enable_importance_scoring);
    }
}
