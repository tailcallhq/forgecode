use std::fmt;

use forge_domain::{Effort, ReasoningConfig};
use serde::{Deserialize, Serialize};

/// OpenRouter-specific effort level.
///
/// Mirrors [`forge_domain::Effort`] but maps [`Effort::Max`] to `"xhigh"`
/// because OpenRouter does not recognise the `"max"` string — its highest
/// supported value is `"xhigh"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenRouterEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    /// Serialises as `"xhigh"`. Also used when the domain effort is `Max`.
    Xhigh,
}

impl fmt::Display for OpenRouterEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
        };
        f.write_str(s)
    }
}

impl From<Effort> for OpenRouterEffort {
    fn from(effort: Effort) -> Self {
        match effort {
            Effort::None => Self::None,
            Effort::Minimal => Self::Minimal,
            Effort::Low => Self::Low,
            Effort::Medium => Self::Medium,
            Effort::High => Self::High,
            // Both XHigh and Max map to "xhigh" — OpenRouter's maximum.
            Effort::XHigh | Effort::Max => Self::Xhigh,
        }
    }
}

/// OpenRouter-specific reasoning configuration.
///
/// Used as the wire type for the `reasoning` field in OpenRouter requests.
/// Mirrors [`forge_domain::ReasoningConfig`] but uses [`OpenRouterEffort`]
/// so that `effort: max` is transparently normalised to `"xhigh"` during JSON
/// serialization. OpenRouter does not recognise `"max"` — `"xhigh"` is its
/// highest supported effort level.
///
/// Built from [`forge_domain::ReasoningConfig`] via `From` in the
/// `From<Context> for Request` conversion, so no transformer step is required.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
// FIXME: Rename to `ReasoningConfig` and move to `openai/request`
pub struct OpenRouterReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<OpenRouterEffort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl From<ReasoningConfig> for OpenRouterReasoningConfig {
    fn from(config: ReasoningConfig) -> Self {
        Self {
            effort: config.effort.map(OpenRouterEffort::from),
            max_tokens: config.max_tokens,
            exclude: config.exclude,
            enabled: config.enabled,
        }
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Effort, ReasoningConfig};
    use pretty_assertions::assert_eq;

    use super::*;

    // ── OpenRouterEffort conversions ──────────────────────────────────────────

    #[test]
    fn test_max_maps_to_xhigh() {
        let actual = OpenRouterEffort::from(Effort::Max);
        let expected = OpenRouterEffort::Xhigh;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_xhigh_maps_to_xhigh() {
        let actual = OpenRouterEffort::from(Effort::XHigh);
        let expected = OpenRouterEffort::Xhigh;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_all_other_efforts_preserved() {
        assert_eq!(OpenRouterEffort::from(Effort::None), OpenRouterEffort::None);
        assert_eq!(OpenRouterEffort::from(Effort::Minimal), OpenRouterEffort::Minimal);
        assert_eq!(OpenRouterEffort::from(Effort::Low), OpenRouterEffort::Low);
        assert_eq!(OpenRouterEffort::from(Effort::Medium), OpenRouterEffort::Medium);
        assert_eq!(OpenRouterEffort::from(Effort::High), OpenRouterEffort::High);
    }

    // ── Display ───────────────────────────────────────────────────────────────

    #[test]
    fn test_display_xhigh() {
        assert_eq!(OpenRouterEffort::Xhigh.to_string(), "xhigh");
    }

    #[test]
    fn test_display_all_variants() {
        assert_eq!(OpenRouterEffort::None.to_string(), "none");
        assert_eq!(OpenRouterEffort::Minimal.to_string(), "minimal");
        assert_eq!(OpenRouterEffort::Low.to_string(), "low");
        assert_eq!(OpenRouterEffort::Medium.to_string(), "medium");
        assert_eq!(OpenRouterEffort::High.to_string(), "high");
    }

    // ── Serialization ─────────────────────────────────────────────────────────

    #[test]
    fn test_xhigh_serializes_as_xhigh_string() {
        let config = OpenRouterReasoningConfig {
            effort: Some(OpenRouterEffort::Xhigh),
            max_tokens: None,
            exclude: None,
            enabled: None,
        };
        let actual = serde_json::to_value(&config).unwrap();
        assert_eq!(actual["effort"], "xhigh");
    }

    #[test]
    fn test_max_to_xhigh_round_trip_serializes_as_xhigh() {
        let domain_config = ReasoningConfig {
            effort: Some(Effort::Max),
            max_tokens: None,
            exclude: None,
            enabled: None,
        };
        let or_config = OpenRouterReasoningConfig::from(domain_config);
        let actual = serde_json::to_value(&or_config).unwrap();
        assert_eq!(actual["effort"], "xhigh");
    }

    #[test]
    fn test_all_fields_preserved_in_conversion() {
        let domain_config = ReasoningConfig {
            effort: Some(Effort::High),
            max_tokens: Some(4000),
            exclude: Some(true),
            enabled: Some(true),
        };
        let actual = serde_json::to_value(OpenRouterReasoningConfig::from(domain_config)).unwrap();
        assert_eq!(actual["effort"], "high");
        assert_eq!(actual["max_tokens"], 4000);
        assert_eq!(actual["exclude"], true);
        assert_eq!(actual["enabled"], true);
    }
}
