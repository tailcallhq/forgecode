use forge_domain::{Effort, Transformer};

use crate::dto::openai::Request;

/// Transformer that converts standard ReasoningConfig to reasoning_effort
/// format
///
/// OpenAI-compatible APIs expect `reasoning_effort` parameter instead of the
/// internal `reasoning` config object.
///
/// # Transformation Rules
///
/// - If `reasoning.enabled == Some(false)` → use "none" (disables reasoning)
/// - If `reasoning.effort` is set → use that value directly
/// - If `reasoning.enabled == Some(true)` but no effort → default to "medium"
/// - Original `reasoning` field is removed after transformation
///
/// # Note
///
/// Budget (`max_tokens`) is not converted to an effort level — effort and token
/// budget are independent concerns.  Pass `effort` explicitly to control
/// reasoning quality.
pub struct SetReasoningEffort;

impl Transformer for SetReasoningEffort {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        // Check if reasoning config exists
        if let Some(reasoning) = request.reasoning.take() {
            let effort = if reasoning.enabled == Some(false) {
                // Disabled - use "none" to disable reasoning
                Some("none".to_string())
            } else if let Some(effort) = reasoning.effort {
                // Use the effort value directly
                Some(effort.to_string())
            } else if reasoning.enabled == Some(true) {
                // Default to "medium" if enabled but no effort specified
                Some(Effort::Medium.to_string())
            } else {
                None
            };

            request.reasoning_effort = effort;
            request.reasoning = None;
        }

        request
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Effort, ReasoningConfig};
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_reasoning_enabled_true_no_effort_defaults_to_medium() {
        let fixture = Request::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            effort: None,
            max_tokens: None,
            exclude: None,
        });

        let mut transformer = SetReasoningEffort;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.reasoning_effort, Some("medium".to_string()));
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_reasoning_enabled_false_converts_to_none() {
        let fixture = Request::default().reasoning(ReasoningConfig {
            enabled: Some(false),
            effort: None,
            max_tokens: None,
            exclude: None,
        });

        let mut transformer = SetReasoningEffort;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.reasoning_effort, Some("none".to_string()));
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_reasoning_with_effort_low() {
        let fixture = Request::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            effort: Some(Effort::Low),
            max_tokens: None,
            exclude: None,
        });

        let mut transformer = SetReasoningEffort;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.reasoning_effort, Some("low".to_string()));
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_reasoning_with_effort_medium() {
        let fixture = Request::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            effort: Some(Effort::Medium),
            max_tokens: None,
            exclude: None,
        });

        let mut transformer = SetReasoningEffort;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.reasoning_effort, Some("medium".to_string()));
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_reasoning_with_effort_high() {
        let fixture = Request::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            effort: Some(Effort::High),
            max_tokens: None,
            exclude: None,
        });

        let mut transformer = SetReasoningEffort;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.reasoning_effort, Some("high".to_string()));
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_reasoning_none_doesnt_add_effort() {
        let fixture = Request::default();

        let mut transformer = SetReasoningEffort;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.reasoning_effort, None);
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_reasoning_enabled_none_doesnt_add_effort() {
        let fixture = Request::default().reasoning(ReasoningConfig {
            enabled: None,
            effort: None,
            max_tokens: None,
            exclude: None,
        });

        let mut transformer = SetReasoningEffort;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.reasoning_effort, None);
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_reasoning_with_budget_defaults_to_medium_effort() {
        // max_tokens (budget) is independent from effort; when only budget is
        // set and enabled=true, the transformer falls back to the default effort.
        let fixture = Request::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            effort: None,
            max_tokens: Some(1024),
            exclude: None,
        });

        let mut transformer = SetReasoningEffort;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.reasoning_effort, Some("medium".to_string()));
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_reasoning_with_budget_high_defaults_to_medium_effort() {
        // Even a large budget does not elevate the effort; use explicit effort instead.
        let fixture = Request::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            effort: None,
            max_tokens: Some(8193),
            exclude: None,
        });

        let mut transformer = SetReasoningEffort;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.reasoning_effort, Some("medium".to_string()));
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_effort_takes_precedence_over_budget() {
        // When both effort and max_tokens are set, effort should take precedence
        let fixture = Request::default().reasoning(ReasoningConfig {
            enabled: Some(true),
            effort: Some(Effort::High),
            max_tokens: Some(1024),
            exclude: None,
        });

        let mut transformer = SetReasoningEffort;
        let actual = transformer.transform(fixture);

        assert_eq!(actual.reasoning_effort, Some("high".to_string()));
        assert_eq!(actual.reasoning, None);
    }
}
