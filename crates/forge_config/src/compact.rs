use std::time::Duration;

use derive_setters::Setters;
use fake::Dummy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Percentage;

/// Frequency at which forge checks for updates
#[derive(Default, Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, fake::Dummy)]
#[serde(rename_all = "snake_case")]
pub enum UpdateFrequency {
    Daily,
    Weekly,
    #[default]
    Always,
}

impl From<UpdateFrequency> for Duration {
    fn from(val: UpdateFrequency) -> Self {
        match val {
            UpdateFrequency::Daily => Duration::from_secs(60 * 60 * 24),
            UpdateFrequency::Weekly => Duration::from_secs(60 * 60 * 24 * 7),
            UpdateFrequency::Always => Duration::ZERO,
        }
    }
}

/// Configuration for automatic forge updates
#[derive(
    Debug, Clone, Serialize, Deserialize, Default, JsonSchema, Setters, PartialEq, fake::Dummy,
)]
#[setters(strip_option, into)]
pub struct Update {
    /// How frequently forge checks for updates
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

    /// Maximum number of tokens before triggering compaction
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_threshold: Option<usize>,

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
            turn_threshold: None,
            message_threshold: None,
            model: None,
            eviction_window: Percentage::new(0.2).unwrap(),
            retention_window: 0,
            on_turn_end: None,
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
            turn_threshold: fake::Faker.fake_with_rng(rng),
            message_threshold: fake::Faker.fake_with_rng(rng),
            model: fake::Faker.fake_with_rng(rng),
            on_turn_end: fake::Faker.fake_with_rng(rng),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
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

        let toml = toml_edit::ser::to_string_pretty(&fixture).unwrap();

        let actual = ConfigReader::default()
            .read_defaults()
            .read_toml(&toml)
            .build()
            .unwrap()
            .compact
            .unwrap_or_default();

        assert_eq!(actual.eviction_window, fixture.eviction_window);
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
}
