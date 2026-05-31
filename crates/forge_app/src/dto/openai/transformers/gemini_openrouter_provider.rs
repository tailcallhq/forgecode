use forge_domain::{ReasoningConfig, Transformer};

use crate::dto::openai::{ProviderPreferences, Request};

/// Sets OpenRouter provider preferences for Gemini models.
pub struct SetGeminiOpenRouterProvider;

impl Transformer for SetGeminiOpenRouterProvider {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        request.provider = Some(ProviderPreferences {
            order: Some(vec!["google-ai-studio".to_string()]),
            ignore: Some(vec!["google-vertex".to_string()]),
        });
        request.temperature = Some(0.7);
        request.top_p = Some(0.95);
        request.top_k = Some(64);
        if let Some(ref mut reasoning) = request.reasoning {
            reasoning.enabled = Some(true);
            request.reasoning = Some(ReasoningConfig {
                enabled: Some(true),
                effort: Some(forge_domain::Effort::High),
                ..reasoning.clone()
            });
        }

        request
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_sets_openrouter_provider_preferences_for_gemini() {
        let fixture = Request::default();
        let mut transformer = SetGeminiOpenRouterProvider;
        let actual = transformer.transform(fixture);
        let expected = Some(ProviderPreferences {
            order: Some(vec!["google-ai-studio".to_string()]),
            ignore: Some(vec!["google-vertex".to_string()]),
        });

        assert_eq!(actual.provider, expected);
    }
}
