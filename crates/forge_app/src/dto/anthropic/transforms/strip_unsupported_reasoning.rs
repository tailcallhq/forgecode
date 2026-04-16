use forge_domain::Transformer;

use crate::dto::anthropic::Request;

/// Strips reasoning parameters (`thinking` and `output_config`) from an
/// Anthropic [`Request`] when the target model does not support them.
///
/// Some Anthropic models (for example, `claude-3-5-haiku`) do not support the
/// `thinking` object or the `output_config.effort` parameter. Sending either
/// field to such models results in a 400 invalid request error.
///
/// This transformer is constructed with the target model ID and removes the
/// unsupported fields before the request is serialized and sent.
///
/// # Model support matrix
///
/// | Model                     | `thinking` | `output_config.effort` |
/// |---------------------------|------------|------------------------|
/// | claude-3-5-haiku          | no         | no                     |
/// | claude-3-haiku            | no         | no                     |
/// | claude-3-opus             | no         | no                     |
/// | claude-3-sonnet           | no         | no                     |
/// | claude-3-5-sonnet         | yes        | no                     |
/// | claude-3-7-sonnet         | yes        | no                     |
/// | claude-sonnet-4           | yes        | yes                    |
/// | claude-opus-4             | yes        | yes                    |
pub struct StripUnsupportedReasoning {
    model: String,
}

impl StripUnsupportedReasoning {
    /// Creates a new transformer for the given model ID.
    ///
    /// # Arguments
    ///
    /// * `model` - Anthropic model identifier used to determine which
    ///   reasoning-related request fields are supported.
    pub fn new(model: impl Into<String>) -> Self {
        Self { model: model.into() }
    }
}

impl Transformer for StripUnsupportedReasoning {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        if request.thinking.is_some() && !model_supports_thinking(&self.model) {
            tracing::debug!(
                model = %self.model,
                "Model does not support thinking; stripping thinking parameter"
            );
            request.thinking = None;
        }
        if request.output_config.is_some() && !model_supports_effort(&self.model) {
            tracing::debug!(
                model = %self.model,
                "Model does not support effort; stripping output_config parameter"
            );
            request.output_config = None;
        }
        request
    }
}

/// Checks whether the given Anthropic model supports extended thinking
/// (the `thinking` object with `budgetTokens`).
///
/// Supported: claude-3-7-sonnet, claude-3-5-sonnet (v2+), claude-sonnet-4,
/// claude-opus-4, and later.
///
/// Not supported: claude-3-5-haiku, claude-3-haiku, claude-3-opus,
/// claude-3-sonnet, and earlier.
fn model_supports_thinking(model: &str) -> bool {
    let model = model.to_lowercase();

    // Claude 4+ generation always supports thinking
    if model.contains("claude-opus-4") || model.contains("claude-sonnet-4") {
        return true;
    }

    // Claude 3.7 Sonnet supports thinking
    if model.contains("claude-3-7") || model.contains("claude-3.7") {
        return true;
    }

    // Claude 3.5 Sonnet (but NOT Haiku) supports thinking
    if (model.contains("claude-3-5") || model.contains("claude-3.5")) && !model.contains("haiku") {
        return true;
    }

    false
}

/// Checks whether the given Anthropic model supports the `output_config.effort`
/// parameter (effort-based reasoning without an explicit budget).
///
/// Only the newest generation of Anthropic models support this:
/// claude-sonnet-4, claude-opus-4, and later.
fn model_supports_effort(model: &str) -> bool {
    let model = model.to_lowercase();

    // Only Claude 4+ generation models support output_config.effort
    model.contains("claude-opus-4") || model.contains("claude-sonnet-4")
}

#[cfg(test)]
mod tests {
    use forge_domain::{Context, Effort, ReasoningConfig, Transformer};
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::dto::anthropic::request::Request;

    // --- model_supports_thinking tests ---

    #[test]
    fn test_model_supports_thinking_claude_3_5_haiku() {
        let actual = model_supports_thinking("claude-3-5-haiku-20241022");
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_model_supports_thinking_claude_3_5_sonnet() {
        let actual = model_supports_thinking("claude-3-5-sonnet-20241022");
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_model_supports_thinking_claude_3_7_sonnet() {
        let actual = model_supports_thinking("claude-3-7-sonnet-20250219");
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_model_supports_thinking_claude_sonnet_4() {
        let actual = model_supports_thinking("claude-sonnet-4-20250514");
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_model_supports_thinking_claude_opus_4() {
        let actual = model_supports_thinking("claude-opus-4-20250415");
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_model_supports_thinking_claude_3_haiku() {
        let actual = model_supports_thinking("claude-3-haiku-20240307");
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_model_supports_thinking_claude_3_opus() {
        let actual = model_supports_thinking("claude-3-opus-20240229");
        let expected = false;
        assert_eq!(actual, expected);
    }

    // --- model_supports_effort tests ---

    #[test]
    fn test_model_supports_effort_claude_3_5_haiku() {
        let actual = model_supports_effort("claude-3-5-haiku-20241022");
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_model_supports_effort_claude_3_5_sonnet() {
        let actual = model_supports_effort("claude-3-5-sonnet-20241022");
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_model_supports_effort_claude_sonnet_4() {
        let actual = model_supports_effort("claude-sonnet-4-20250514");
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_model_supports_effort_claude_opus_4() {
        let actual = model_supports_effort("claude-opus-4-20250415");
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_model_supports_effort_claude_3_7_sonnet() {
        let actual = model_supports_effort("claude-3-7-sonnet-20250219");
        let expected = false;
        assert_eq!(actual, expected);
    }

    // --- StripUnsupportedReasoning transform tests ---

    #[test]
    fn test_strip_reasoning_haiku_strips_output_config() {
        let mut transformer = StripUnsupportedReasoning::new("claude-3-5-haiku-20241022");
        let fixture = Request::try_from(Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: None,
            effort: Some(Effort::High),
            exclude: None,
        }))
        .unwrap();

        assert!(
            fixture.output_config.is_some(),
            "output_config should be set before stripping"
        );

        let actual = transformer.transform(fixture);

        assert_eq!(
            actual.output_config, None,
            "haiku should not have output_config"
        );
        assert_eq!(actual.thinking, None, "haiku should not have thinking");
    }

    #[test]
    fn test_strip_reasoning_haiku_strips_thinking() {
        let mut transformer = StripUnsupportedReasoning::new("claude-3-5-haiku-20241022");
        let fixture = Request::try_from(Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: Some(10000),
            effort: None,
            exclude: None,
        }))
        .unwrap();

        assert!(
            fixture.thinking.is_some(),
            "thinking should be set before stripping"
        );

        let actual = transformer.transform(fixture);

        assert_eq!(actual.thinking, None, "haiku should not have thinking");
    }

    #[test]
    fn test_strip_reasoning_sonnet_4_keeps_output_config() {
        let mut transformer = StripUnsupportedReasoning::new("claude-sonnet-4-20250514");
        let fixture = Request::try_from(Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: None,
            effort: Some(Effort::High),
            exclude: None,
        }))
        .unwrap();

        let expected = fixture.output_config.clone();
        let actual = transformer.transform(fixture);

        assert_eq!(
            actual.output_config, expected,
            "sonnet-4 should keep output_config"
        );
    }

    #[test]
    fn test_strip_reasoning_sonnet_4_keeps_thinking() {
        let mut transformer = StripUnsupportedReasoning::new("claude-sonnet-4-20250514");
        let fixture = Request::try_from(Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: Some(10000),
            effort: None,
            exclude: None,
        }))
        .unwrap();

        let expected = fixture.thinking.clone();
        let actual = transformer.transform(fixture);

        assert_eq!(actual.thinking, expected, "sonnet-4 should keep thinking");
    }

    #[test]
    fn test_strip_reasoning_3_7_sonnet_keeps_thinking_strips_effort() {
        let mut transformer = StripUnsupportedReasoning::new("claude-3-7-sonnet-20250219");
        let fixture = Request::try_from(Context::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            max_tokens: None,
            effort: Some(Effort::Medium),
            exclude: None,
        }))
        .unwrap();

        // 3.7 sonnet supports thinking but NOT output_config.effort
        assert!(
            fixture.output_config.is_some(),
            "output_config should be set before stripping"
        );

        let actual = transformer.transform(fixture);

        assert_eq!(
            actual.output_config, None,
            "3.7 sonnet should not have output_config"
        );
    }
}
