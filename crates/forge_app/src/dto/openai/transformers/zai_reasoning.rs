use forge_domain::Transformer;

use crate::dto::openai::{Request, ThinkingConfig, ThinkingType};

/// Transformer that converts standard ReasoningConfig to z.ai's thinking format
///
/// Z.ai providers require reasoning to be set as `"thinking": {"type":
/// "enabled"}` while the codebase uses OpenAI's `reasoning` field with
/// ReasoningConfig structure. This transformer maps the standard reasoning
/// configuration to z.ai's format.
///
/// # Transformation Rules
///
/// - If `reasoning.enabled == Some(true)` → `thinking = {"type": "enabled"}`
/// - If `reasoning.enabled == Some(false)` → `thinking = {"type": "disabled"}`
/// - If `reasoning.enabled == None` and `reasoning.effort ==
///   Some(Effort::None)` → `thinking = {"type": "disabled"}`. The orchestrator
///   preserves this opt-out for z.ai providers so it can be mapped here.
/// - If `reasoning.enabled == None` and `reasoning.effort` is any other value →
///   `thinking = {"type": "enabled"}`
/// - If `reasoning` is None or both `enabled` and `effort` are None → no
///   `thinking` field added
/// - Original `reasoning` field is removed after transformation
///
/// # Note
///
/// Z.ai only supports enabled/disabled state. `effort` is mapped to that
/// on/off state when `enabled` is unset. Other ReasoningConfig fields
/// (`max_tokens`, `exclude`) are ignored as they are not supported by z.ai's
/// API.
pub struct SetZaiThinking;

impl Transformer for SetZaiThinking {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        if let Some(reasoning) = request.reasoning.take() {
            // Effort::None is a strong opt-out and wins over `enabled`, matching
            // Context::is_reasoning_supported's resolution order in
            // forge_domain. Without this, an explicit `:reasoning-effort none`
            // followed later by `enabled: true` (e.g. from defaults) would
            // re-enable thinking, contradicting the user's most recent intent.
            let enabled = if matches!(reasoning.effort, Some(forge_domain::Effort::None)) {
                Some(false)
            } else if reasoning.enabled.is_some() {
                reasoning.enabled
            } else {
                reasoning.effort.map(|_| true)
            };

            if let Some(enabled) = enabled {
                request.thinking = Some(ThinkingConfig {
                    r#type: if enabled {
                        ThinkingType::Enabled
                    } else {
                        ThinkingType::Disabled
                    },
                });
            }
        }

        request
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_reasoning_enabled_true_converts_to_thinking_enabled() {
        let fixture = Request::default().reasoning(forge_domain::ReasoningConfig {
            enabled: Some(true),
            effort: None,
            max_tokens: None,
            exclude: None,
        });

        let mut transformer = SetZaiThinking;
        let actual = transformer.transform(fixture);

        let expected_thinking = Some(ThinkingConfig { r#type: ThinkingType::Enabled });
        assert_eq!(actual.thinking, expected_thinking);
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_reasoning_enabled_false_converts_to_thinking_disabled() {
        let fixture = Request::default().reasoning(forge_domain::ReasoningConfig {
            enabled: Some(false),
            effort: None,
            max_tokens: None,
            exclude: None,
        });

        let mut transformer = SetZaiThinking;
        let actual = transformer.transform(fixture);

        let expected_thinking = Some(ThinkingConfig { r#type: ThinkingType::Disabled });
        assert_eq!(actual.thinking, expected_thinking);
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_reasoning_none_doesnt_add_thinking() {
        let fixture = Request::default();

        let mut transformer = SetZaiThinking;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.thinking, None);
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_reasoning_enabled_none_doesnt_add_thinking() {
        let fixture = Request::default().reasoning(forge_domain::ReasoningConfig {
            enabled: None,
            effort: None,
            max_tokens: None,
            exclude: None,
        });

        let mut transformer = SetZaiThinking;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.thinking, None);
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_effort_none_overrides_enabled_true() {
        // Matches Context::is_reasoning_supported precedence: Effort::None wins
        // over enabled: true. Otherwise an opt-out at the effort level would
        // be silently re-enabled by a stale `enabled: true` from defaults.
        let fixture = Request::default().reasoning(forge_domain::ReasoningConfig {
            enabled: Some(true),
            effort: Some(forge_domain::Effort::None),
            max_tokens: None,
            exclude: None,
        });

        let mut transformer = SetZaiThinking;
        let actual = transformer.transform(fixture);

        let expected_thinking = Some(ThinkingConfig { r#type: ThinkingType::Disabled });
        assert_eq!(actual.thinking, expected_thinking);
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_reasoning_effort_none_converts_to_thinking_disabled() {
        let fixture = Request::default().reasoning(forge_domain::ReasoningConfig {
            enabled: None,
            effort: Some(forge_domain::Effort::None),
            max_tokens: None,
            exclude: None,
        });

        let mut transformer = SetZaiThinking;
        let actual = transformer.transform(fixture);

        let expected_thinking = Some(ThinkingConfig { r#type: ThinkingType::Disabled });
        assert_eq!(actual.thinking, expected_thinking);
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_glm5_effort_none_passthrough_converts_to_thinking_disabled() {
        let fixture = Request::default()
            .model(forge_domain::ModelId::new("glm-5"))
            .reasoning(forge_domain::ReasoningConfig {
                enabled: None,
                effort: Some(forge_domain::Effort::None),
                max_tokens: None,
                exclude: None,
            });

        let mut transformer = SetZaiThinking;
        let actual = transformer.transform(fixture);

        let expected_thinking = Some(ThinkingConfig { r#type: ThinkingType::Disabled });
        assert_eq!(actual.thinking, expected_thinking);
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_reasoning_effort_high_converts_to_thinking_enabled() {
        let fixture = Request::default().reasoning(forge_domain::ReasoningConfig {
            enabled: None,
            effort: Some(forge_domain::Effort::High),
            max_tokens: None,
            exclude: None,
        });

        let mut transformer = SetZaiThinking;
        let actual = transformer.transform(fixture);

        let expected_thinking = Some(ThinkingConfig { r#type: ThinkingType::Enabled });
        assert_eq!(actual.thinking, expected_thinking);
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_reasoning_with_max_tokens_ignores_max_tokens() {
        let fixture = Request::default().reasoning(forge_domain::ReasoningConfig {
            enabled: Some(true),
            effort: None,
            max_tokens: Some(2048),
            exclude: None,
        });

        let mut transformer = SetZaiThinking;
        let actual = transformer.transform(fixture);

        let expected_thinking = Some(ThinkingConfig { r#type: ThinkingType::Enabled });
        assert_eq!(actual.thinking, expected_thinking);
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_reasoning_with_effort_ignores_effort() {
        let fixture = Request::default().reasoning(forge_domain::ReasoningConfig {
            enabled: Some(true),
            effort: Some(forge_domain::Effort::High),
            max_tokens: None,
            exclude: None,
        });

        let mut transformer = SetZaiThinking;
        let actual = transformer.transform(fixture);

        let expected_thinking = Some(ThinkingConfig { r#type: ThinkingType::Enabled });
        assert_eq!(actual.thinking, expected_thinking);
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_reasoning_with_exclude_ignores_exclude() {
        let fixture = Request::default().reasoning(forge_domain::ReasoningConfig {
            enabled: Some(true),
            effort: None,
            max_tokens: None,
            exclude: Some(true),
        });

        let mut transformer = SetZaiThinking;
        let actual = transformer.transform(fixture);

        let expected_thinking = Some(ThinkingConfig { r#type: ThinkingType::Enabled });
        assert_eq!(actual.thinking, expected_thinking);
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_thinking_config_serialization() {
        let thinking = ThinkingConfig { r#type: ThinkingType::Enabled };
        let actual = serde_json::to_string(&thinking).unwrap();
        let expected = r#"{"type":"enabled"}"#;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_thinking_config_deserialization() {
        let json = r#"{"type":"disabled"}"#;
        let actual: ThinkingConfig = serde_json::from_str(json).unwrap();
        let expected = ThinkingConfig { r#type: ThinkingType::Disabled };
        assert_eq!(actual, expected);
    }
}
