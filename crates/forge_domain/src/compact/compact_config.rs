use derive_setters::Setters;
use merge::Merge;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ModelId;

/// Per-agent summarizer config consumed by the projector at
/// request-build. Triggers fire when any threshold is met; the
/// sliding window keeps the last N rendered summary frames.
#[derive(Debug, Clone, Serialize, Deserialize, Merge, Setters, JsonSchema, PartialEq)]
#[setters(strip_option, into)]
pub struct Compact {
    /// Forbids a flush when fewer than this many canonical messages
    /// would remain after it, preserving the recent tail verbatim.
    /// `None` means no retention — consumers read via
    /// `effective_retention_window`.
    #[merge(strategy = crate::merge::option)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_window: Option<usize>,

    /// Absolute token cap above which the summarizer fires. Combined
    /// with `token_threshold_percentage` by taking the lower value.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[merge(strategy = crate::merge::option)]
    pub token_threshold: Option<usize>,

    /// Fraction of the model's context window above which the
    /// summarizer fires. Combined with `token_threshold` by taking
    /// the lower value.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_percentage"
    )]
    #[merge(strategy = crate::merge::option)]
    pub token_threshold_percentage: Option<f64>,

    /// Fires the summarizer once the user-role message count in the
    /// assembled request reaches this threshold.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[merge(strategy = crate::merge::option)]
    pub turn_threshold: Option<usize>,

    /// Fires the summarizer once the total message count in the
    /// assembled request reaches this threshold.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[merge(strategy = crate::merge::option)]
    pub message_threshold: Option<usize>,

    /// Overrides the agent's primary model for summary rendering so
    /// a cheaper or faster model can handle summarization.
    #[merge(strategy = crate::merge::option)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelId>,

    /// Fires one summary per projection when the assembled request's
    /// tail is a user message. Independent of budget thresholds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[merge(strategy = crate::merge::option)]
    pub on_turn_end: Option<bool>,

    /// Cap on summary frames the summarizer prepends. Older frames
    /// slide off (lossy true-sliding) when the cap is exceeded;
    /// `None` uses `DEFAULT_MAX_PREPENDED_SUMMARIES` at runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[merge(strategy = crate::merge::option)]
    pub max_prepended_summaries: Option<usize>,
}

/// Runtime fallback for `Compact::max_prepended_summaries` — two
/// frames keeps the last two summarization events visible without
/// bloating the request head.
pub const DEFAULT_MAX_PREPENDED_SUMMARIES: usize = 2;

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
    /// All thresholds unset — the projector falls through to passthrough
    /// until the caller dials a threshold in.
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

    /// Resolves the sliding-window cap to its configured value or
    /// `DEFAULT_MAX_PREPENDED_SUMMARIES` when unset.
    pub fn effective_max_prepended_summaries(&self) -> usize {
        self.max_prepended_summaries
            .unwrap_or(DEFAULT_MAX_PREPENDED_SUMMARIES)
    }

    /// Resolves the tail-preservation count to its configured value or
    /// `0` (no retention) when unset.
    pub fn effective_retention_window(&self) -> usize {
        self.retention_window.unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// Setters leave `model` at `None` so the agent merge later fills it
    /// from the primary model; thresholds set via setters round-trip
    /// verbatim.
    #[test]
    fn test_compact_new_and_setters_leave_model_unset() {
        let compact = Compact::new()
            .token_threshold(1000_usize)
            .turn_threshold(5_usize);

        assert_eq!(compact.model, None);
        assert_eq!(compact.token_threshold, Some(1000_usize));
        assert_eq!(compact.turn_threshold, Some(5_usize));
    }
}
