use forge_domain::{Context, Effort, ReasoningConfig, Transformer};
use tracing::warn;

/// Default budget applied when converting adaptive-style reasoning into legacy
/// budget-based reasoning for pre-4.6 Anthropic model families.
const DEFAULT_LEGACY_BUDGET_TOKENS: usize = 10000;

#[derive(Debug, PartialEq, Eq)]
enum AnthropicModelFamily {
    AdaptiveOnly,
    AdaptiveFriendly,
    LegacyWithEffort,
    LegacyNoEffort,
}

/// Normalizes reasoning knobs for Anthropic model families before provider
/// conversion.
pub(crate) struct ModelSpecificReasoning {
    model_id: String,
}

impl ModelSpecificReasoning {
    /// Creates a model-specific reasoning normalizer for the given model id.
    pub(crate) fn new(model_id: impl Into<String>) -> Self {
        Self { model_id: model_id.into() }
    }

    fn family(&self) -> AnthropicModelFamily {
        let id = self.model_id.to_lowercase();
        if id.contains("opus-4-7") || id.contains("47-opus") {
            AnthropicModelFamily::AdaptiveOnly
        } else if id.contains("opus-4-6")
            || id.contains("46-opus")
            || id.contains("sonnet-4-6")
            || id.contains("46-sonnet")
        {
            AnthropicModelFamily::AdaptiveFriendly
        } else if id.contains("opus-4-5") || id.contains("45-opus") {
            AnthropicModelFamily::LegacyWithEffort
        } else {
            AnthropicModelFamily::LegacyNoEffort
        }
    }
}

fn replace_xhigh_with_max(reasoning: &mut Option<ReasoningConfig>) {
    if let Some(reasoning) = reasoning.as_mut()
        && reasoning.effort == Some(Effort::XHigh)
    {
        reasoning.effort = Some(Effort::Max);
    }
}

fn clamp_effort_to_high(reasoning: &mut Option<ReasoningConfig>) {
    if let Some(reasoning) = reasoning.as_mut()
        && matches!(reasoning.effort, Some(Effort::XHigh | Effort::Max))
    {
        reasoning.effort = Some(Effort::High);
    }
}

fn set_default_legacy_budget(reasoning: &mut Option<ReasoningConfig>) {
    if let Some(reasoning) = reasoning.as_mut()
        && reasoning.max_tokens.is_none()
    {
        reasoning.max_tokens = Some(DEFAULT_LEGACY_BUDGET_TOKENS);
    }
}

impl Transformer for ModelSpecificReasoning {
    type Value = Context;

    fn transform(&mut self, mut context: Self::Value) -> Self::Value {
        let reasoning_on = context.is_reasoning_supported();

        match self.family() {
            AnthropicModelFamily::AdaptiveOnly => {
                if reasoning_on
                    && let Some(reasoning) = context.reasoning.as_mut()
                    && let Some(max_tokens) = reasoning.max_tokens.take()
                {
                    warn!(
                        model = %self.model_id,
                        dropped_max_tokens = max_tokens,
                        "Dropping `reasoning.max_tokens` for Opus 4.7: extended thinking budgets are unsupported. Use `reasoning.effort` to control thinking depth instead."
                    );
                }
                context.temperature = None;
                context.top_p = None;
                context.top_k = None;
            }
            AnthropicModelFamily::AdaptiveFriendly => {
                if reasoning_on {
                    replace_xhigh_with_max(&mut context.reasoning);
                }
            }
            AnthropicModelFamily::LegacyWithEffort => {
                if reasoning_on {
                    set_default_legacy_budget(&mut context.reasoning);
                    clamp_effort_to_high(&mut context.reasoning);
                }
            }
            AnthropicModelFamily::LegacyNoEffort => {
                if reasoning_on {
                    set_default_legacy_budget(&mut context.reasoning);
                    if let Some(reasoning) = context.reasoning.as_mut()
                        && reasoning.effort.is_some()
                    {
                        warn!(
                            model = %self.model_id,
                            "Dropping `reasoning.effort`: the effort parameter is only supported on Opus 4.5, Opus 4.6, Sonnet 4.6, and Opus 4.7."
                        );
                        reasoning.effort = None;
                    }
                }
            }
        }

        context
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Context, Effort, ReasoningConfig, Temperature, TopK, TopP, Transformer};
    use pretty_assertions::assert_eq;

    use super::*;

    fn fixture_context_with_sampling() -> Context {
        Context::default()
            .temperature(Temperature::new(0.5).unwrap())
            .top_p(TopP::new(0.9).unwrap())
            .top_k(TopK::new(40).unwrap())
    }

    #[test]
    fn test_opus_4_7_drops_max_tokens_and_sampling_params() {
        let fixture = fixture_context_with_sampling().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: Some(8000),
            effort: Some(Effort::XHigh),
            exclude: Some(true),
        });

        let actual = ModelSpecificReasoning::new("claude-opus-4-7").transform(fixture);

        let expected = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: None,
            effort: Some(Effort::XHigh),
            exclude: Some(true),
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_opus_4_7_strips_sampling_even_without_reasoning() {
        let fixture = fixture_context_with_sampling();

        let actual = ModelSpecificReasoning::new("claude-opus-4-7").transform(fixture);

        let expected = Context::default();

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_adaptive_friendly_replaces_xhigh_with_max() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: None,
            effort: Some(Effort::XHigh),
            exclude: None,
        });

        let actual = ModelSpecificReasoning::new("claude-opus-4-6").transform(fixture);

        let expected = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: None,
            effort: Some(Effort::Max),
            exclude: None,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_legacy_with_effort_backfills_budget_and_clamps_effort() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: None,
            effort: Some(Effort::Max),
            exclude: None,
        });

        let actual = ModelSpecificReasoning::new("claude-opus-4-5-20251101").transform(fixture);

        let expected = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: Some(DEFAULT_LEGACY_BUDGET_TOKENS),
            effort: Some(Effort::High),
            exclude: None,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_legacy_no_effort_backfills_budget_and_drops_effort() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: None,
            effort: Some(Effort::High),
            exclude: None,
        });

        let actual = ModelSpecificReasoning::new("claude-3-7-sonnet-20250219").transform(fixture);

        let expected = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: Some(DEFAULT_LEGACY_BUDGET_TOKENS),
            effort: None,
            exclude: None,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_opus_4_7_bedrock_prefix_still_matches() {
        // Bedrock region prefixes (`us.anthropic.claude-...`) must still be
        // classified as AdaptiveOnly so sampling params are stripped and
        // `max_tokens` is dropped.
        let fixture = fixture_context_with_sampling().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: Some(8000),
            effort: Some(Effort::XHigh),
            exclude: None,
        });

        let actual = ModelSpecificReasoning::new("us.anthropic.claude-opus-4-7").transform(fixture);

        let expected = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: None,
            effort: Some(Effort::XHigh),
            exclude: None,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_opus_4_7_preserves_effort_when_dropping_max_tokens() {
        // When both knobs are set on 4.7, only `max_tokens` should be dropped;
        // `effort` is the remaining depth knob and must survive.
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: Some(8000),
            effort: Some(Effort::XHigh),
            exclude: None,
        });

        let actual = ModelSpecificReasoning::new("claude-opus-4-7").transform(fixture);

        let expected = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: None,
            effort: Some(Effort::XHigh),
            exclude: None,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_opus_4_5_clamps_max_to_high() {
        let fixture = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: Some(8000),
            effort: Some(Effort::Max),
            exclude: None,
        });

        let actual = ModelSpecificReasoning::new("claude-opus-4-5-20251101").transform(fixture);

        let expected = Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: Some(8000),
            effort: Some(Effort::High),
            exclude: None,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_opus_4_5_preserves_supported_effort_levels() {
        for level in [Effort::Low, Effort::Medium, Effort::High] {
            let fixture = Context::default().reasoning(ReasoningConfig {
                enabled: Some(true),
                max_tokens: Some(8000),
                effort: Some(level.clone()),
                exclude: None,
            });

            let actual = ModelSpecificReasoning::new("claude-opus-4-5-20251101").transform(fixture);

            let expected = Context::default().reasoning(ReasoningConfig {
                enabled: Some(true),
                max_tokens: Some(8000),
                effort: Some(level.clone()),
                exclude: None,
            });

            assert_eq!(actual, expected, "effort level {:?}", level);
        }
    }

    #[test]
    fn test_legacy_no_effort_drops_effort_for_all_pre_4_5_ids() {
        // All pre-Opus-4.5 Claude ids (plus the newer non-effort family members
        // Sonnet 4.5 and Haiku 4.5) should land in LegacyNoEffort and have their
        // effort stripped.
        for model in [
            "claude-sonnet-4-5-20250929",
            "claude-haiku-4-5-20251001",
            "claude-opus-4-1-20250805",
            "claude-opus-4-20250514",
            "claude-3-7-sonnet-20250219",
        ] {
            let fixture = Context::default().reasoning(ReasoningConfig {
                enabled: Some(true),
                max_tokens: Some(8000),
                effort: Some(Effort::High),
                exclude: None,
            });

            let actual = ModelSpecificReasoning::new(model).transform(fixture);

            let expected = Context::default().reasoning(ReasoningConfig {
                enabled: Some(true),
                max_tokens: Some(8000),
                effort: None,
                exclude: None,
            });

            assert_eq!(actual, expected, "model {}", model);
        }
    }

    #[test]
    fn test_no_reasoning_is_preserved_everywhere() {
        // A context without `reasoning` must pass through unchanged for every
        // family except AdaptiveOnly, which still strips sampling params.
        for model in [
            "claude-opus-4-6",
            "claude-sonnet-4-6",
            "claude-opus-4-5-20251101",
            "claude-3-7-sonnet-20250219",
        ] {
            let fixture = Context::default();
            let actual = ModelSpecificReasoning::new(model).transform(fixture);
            let expected = Context::default();
            assert_eq!(actual, expected, "model {}", model);
        }
    }

    #[test]
    fn test_adaptive_friendly_preserves_non_xhigh_effort() {
        for level in [Effort::Low, Effort::Medium, Effort::High, Effort::Max] {
            let fixture = Context::default().reasoning(ReasoningConfig {
                enabled: Some(true),
                max_tokens: None,
                effort: Some(level.clone()),
                exclude: None,
            });

            let actual = ModelSpecificReasoning::new("claude-opus-4-6").transform(fixture);

            let expected = Context::default().reasoning(ReasoningConfig {
                enabled: Some(true),
                max_tokens: None,
                effort: Some(level.clone()),
                exclude: None,
            });

            assert_eq!(actual, expected, "effort level {:?}", level);
        }
    }
}
