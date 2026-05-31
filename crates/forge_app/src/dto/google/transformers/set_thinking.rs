use forge_domain::Transformer;

use crate::dto::google::{Level, Request};

pub struct SetThinking {
    pub model_id: String,
}

impl SetThinking {
    pub fn new(model_id: impl Into<String>) -> Self {
        Self { model_id: model_id.into() }
    }
}

impl Transformer for SetThinking {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        // Check if thinking_config exists (which means reasoning was enabled)
        if let Some(generation_config) = &mut request.generation_config
            && let Some(thinking_config) = &mut generation_config.thinking_config
        {
            // Always set include_thoughts to true if thinking config exists
            thinking_config.include_thoughts = Some(true);

            // If model is gemini-3, set thinking level to High
            if self.model_id.contains("gemini-3") {
                thinking_config.thinking_level = Some(Level::High);
            }
        }

        request
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Context, ReasoningConfig};

    use super::*;

    #[test]
    fn test_set_thinking_gemini_3() {
        let context = Context {
            reasoning: Some(ReasoningConfig {
                enabled: Some(true),
                max_tokens: Some(1024),
                ..Default::default()
            }),
            ..Default::default()
        };

        let request = Request::from(context);
        let mut transformer = SetThinking::new("gemini-3.0-pro");
        let transformed = transformer.transform(request);

        let thinking_config = transformed
            .generation_config
            .unwrap()
            .thinking_config
            .unwrap();

        assert_eq!(thinking_config.include_thoughts, Some(true));
        // Should be High for gemini-3
        assert!(matches!(thinking_config.thinking_level, Some(Level::High)));
    }

    #[test]
    fn test_set_thinking_gemini_2() {
        let context = Context {
            reasoning: Some(ReasoningConfig {
                enabled: Some(true),
                max_tokens: Some(1024),
                ..Default::default()
            }),
            ..Default::default()
        };

        let request = Request::from(context);
        let mut transformer = SetThinking::new("gemini-2.0-flash");
        let transformed = transformer.transform(request);

        let thinking_config = transformed
            .generation_config
            .unwrap()
            .thinking_config
            .unwrap();

        assert_eq!(thinking_config.include_thoughts, Some(true));
        // Should be None (unspecified) for non-gemini-3
        assert!(thinking_config.thinking_level.is_none());
    }
}
