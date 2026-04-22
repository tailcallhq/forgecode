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

/// Workflow-level summarizer defaults. Merged into each agent's
/// `forge_domain::Compact` at run time so unset agent fields inherit
/// these values.
#[derive(Debug, Clone, Serialize, Deserialize, Setters, JsonSchema, PartialEq)]
#[setters(strip_option, into)]
pub struct Compact {
    /// Forbids a flush when fewer than this many canonical messages
    /// would remain after it, preserving the recent tail verbatim.
    /// `None` means no retention.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_window: Option<usize>,

    /// Absolute token cap above which the summarizer fires. Combined
    /// with `token_threshold_percentage` by taking the lower value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_threshold: Option<usize>,

    /// Fraction of the model's context window above which the
    /// summarizer fires. Combined with `token_threshold` by taking
    /// the lower value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_threshold_percentage: Option<Percentage>,

    /// Fires the summarizer once the user-role message count in the
    /// assembled request reaches this threshold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_threshold: Option<usize>,

    /// Fires the summarizer once the total message count in the
    /// assembled request reaches this threshold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_threshold: Option<usize>,

    /// Overrides the agent's primary model for summary rendering so
    /// a cheaper or faster model can handle summarization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Fires one summary per projection when the assembled request's
    /// tail is a user message. Independent of budget thresholds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_turn_end: Option<bool>,

    /// Cap on summary frames the summarizer prepends; older frames
    /// slide off when exceeded. `None` uses the runtime default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_prepended_summaries: Option<usize>,
}

impl Default for Compact {
    fn default() -> Self {
        Self::new()
    }
}

impl Compact {
    /// All fields unset so the domain `Compact` merge keeps the
    /// agent's own values wherever the agent configured them.
    pub fn new() -> Self {
        Self {
            token_threshold: None,
            token_threshold_percentage: None,
            turn_threshold: None,
            message_threshold: None,
            model: None,
            retention_window: None,
            on_turn_end: None,
            max_prepended_summaries: None,
        }
    }
}

impl Dummy<fake::Faker> for Compact {
    fn dummy_with_rng<R: fake::RngExt + ?Sized>(_: &fake::Faker, rng: &mut R) -> Self {
        use fake::Fake;
        Self {
            retention_window: fake::Faker.fake_with_rng(rng),
            token_threshold: fake::Faker.fake_with_rng(rng),
            token_threshold_percentage: fake::Faker.fake_with_rng(rng),
            turn_threshold: fake::Faker.fake_with_rng(rng),
            message_threshold: fake::Faker.fake_with_rng(rng),
            model: fake::Faker.fake_with_rng(rng),
            on_turn_end: fake::Faker.fake_with_rng(rng),
            max_prepended_summaries: fake::Faker.fake_with_rng(rng),
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
}
