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
/// - If `reasoning` is None or `enabled` is None → no `thinking` field added
/// - Original `reasoning` field is removed after transformation
///
/// # Note
///
/// Z.ai only supports enabled/disabled state. Other ReasoningConfig fields
/// (`max_tokens`, `effort`, `exclude`) are ignored as they are not supported by
/// z.ai's API.
pub struct SetZaiThinking;

impl Transformer for SetZaiThinking {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        // Check if reasoning config exists and has enabled field set
        if let Some(reasoning) = request.reasoning.take()
            && let Some(enabled) = reasoning.enabled
        {
            request.thinking = Some(ThinkingConfig {
                r#type: if enabled {
                    ThinkingType::Enabled
                } else {
                    ThinkingType::Disabled
                },
            });
            request.reasoning_or = None;
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
