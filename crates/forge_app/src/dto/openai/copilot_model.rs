use forge_domain::ModelId;
use serde::{Deserialize, Serialize};

/// Response returned by the GitHub Copilot `/models` endpoint.
#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct CopilotListModelResponse {
    pub data: Vec<CopilotModel>,
}

/// A model entry as returned by the GitHub Copilot `/models` endpoint.
///
/// GitHub Copilot uses a schema that differs from the standard OpenAI
/// models response: capabilities, limits and access policy are nested
/// under dedicated objects.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CopilotModel {
    pub id: ModelId,
    pub name: Option<String>,
    /// Whether the model is shown in the model picker of official clients
    #[serde(default)]
    pub model_picker_enabled: bool,
    /// Access policy for the model; models with a `disabled` state cannot
    /// be used by the authenticated account
    pub policy: Option<CopilotPolicy>,
    pub capabilities: Option<CopilotCapabilities>,
    pub supported_endpoints: Option<Vec<String>>,
    pub vendor: Option<String>,
    pub preview: Option<bool>,
}

/// Access policy attached to a Copilot model.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct CopilotPolicy {
    pub state: Option<String>,
}

/// Capabilities of a Copilot model.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct CopilotCapabilities {
    /// Model type, e.g. `chat` or `embeddings`
    #[serde(rename = "type")]
    pub model_type: Option<String>,
    pub limits: Option<CopilotLimits>,
    pub supports: Option<CopilotSupports>,
}

/// Token and vision limits of a Copilot model.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct CopilotLimits {
    pub max_context_window_tokens: Option<u64>,
    pub max_prompt_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub vision: Option<CopilotVisionLimits>,
}

/// Vision-specific limits of a Copilot model.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct CopilotVisionLimits {
    pub supported_media_types: Option<Vec<String>>,
}

/// Feature support flags of a Copilot model.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct CopilotSupports {
    pub tool_calls: Option<bool>,
    pub parallel_tool_calls: Option<bool>,
    pub streaming: Option<bool>,
    pub vision: Option<bool>,
    pub adaptive_thinking: Option<bool>,
    pub max_thinking_budget: Option<u64>,
    pub min_thinking_budget: Option<u64>,
    pub reasoning_effort: Option<Vec<String>>,
}

impl CopilotModel {
    /// Returns true when the model can actually be used for chat by the
    /// authenticated account.
    ///
    /// Filters out models that are policy-disabled for the current
    /// subscription (requests to them fail with `model_not_supported`) as
    /// well as non-chat models such as embeddings.
    pub fn is_usable(&self) -> bool {
        // Models explicitly disabled by policy cannot be used and return
        // `model_not_supported` errors when requested
        let policy_disabled = self
            .policy
            .as_ref()
            .and_then(|p| p.state.as_deref())
            .is_some_and(|state| state == "disabled");
        if policy_disabled {
            return false;
        }

        let Some(capabilities) = self.capabilities.as_ref() else {
            return false;
        };

        // Only chat models are usable for conversations (excludes embeddings)
        if capabilities.model_type.as_deref() != Some("chat") {
            return false;
        }

        // Require known token limits and tool call information, mirroring the
        // checks official clients perform before offering a model
        let has_limits = capabilities.limits.as_ref().is_some_and(|limits| {
            limits.max_output_tokens.is_some() && limits.max_prompt_tokens.is_some()
        });
        let has_tool_info = capabilities
            .supports
            .as_ref()
            .is_some_and(|supports| supports.tool_calls.is_some());

        has_limits && has_tool_info
    }
}

impl From<CopilotModel> for forge_domain::Model {
    fn from(value: CopilotModel) -> Self {
        let supports = value
            .capabilities
            .as_ref()
            .and_then(|c| c.supports.as_ref());
        let limits = value.capabilities.as_ref().and_then(|c| c.limits.as_ref());

        let context_length =
            limits.and_then(|l| l.max_context_window_tokens.or(l.max_prompt_tokens));

        let tools_supported = supports.and_then(|s| s.tool_calls);
        let supports_parallel_tool_calls = supports.and_then(|s| s.parallel_tool_calls);

        // A model supports reasoning when it advertises adaptive thinking,
        // reasoning effort levels or a thinking budget
        let supports_reasoning = supports.map(|s| {
            s.adaptive_thinking.unwrap_or(false)
                || s.reasoning_effort
                    .as_ref()
                    .is_some_and(|efforts| !efforts.is_empty())
                || s.max_thinking_budget.is_some()
        });

        let supports_image = supports.and_then(|s| s.vision).unwrap_or(false)
            || limits
                .and_then(|l| l.vision.as_ref())
                .and_then(|v| v.supported_media_types.as_ref())
                .is_some_and(|types| types.iter().any(|t| t.starts_with("image/")));

        let mut input_modalities = vec![forge_domain::InputModality::Text];
        if supports_image {
            input_modalities.push(forge_domain::InputModality::Image);
        }

        forge_domain::Model {
            id: value.id,
            name: value.name,
            description: None,
            context_length,
            tools_supported,
            supports_parallel_tool_calls,
            supports_reasoning,
            input_modalities,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn fixture_chat_model() -> CopilotModel {
        serde_json::from_value(serde_json::json!({
            "id": "claude-sonnet-4.6",
            "name": "Claude Sonnet 4.6",
            "model_picker_enabled": true,
            "policy": { "state": "enabled" },
            "vendor": "Anthropic",
            "preview": false,
            "supported_endpoints": ["/chat/completions", "/v1/messages"],
            "capabilities": {
                "type": "chat",
                "family": "claude-sonnet-4.6",
                "limits": {
                    "max_context_window_tokens": 264000,
                    "max_output_tokens": 64000,
                    "max_prompt_tokens": 200000,
                    "vision": {
                        "max_prompt_image_size": 3145728,
                        "max_prompt_images": 5,
                        "supported_media_types": ["image/jpeg", "image/png"]
                    }
                },
                "supports": {
                    "adaptive_thinking": true,
                    "max_thinking_budget": 32000,
                    "min_thinking_budget": 1024,
                    "parallel_tool_calls": true,
                    "reasoning_effort": ["low", "medium", "high", "max"],
                    "streaming": true,
                    "structured_outputs": true,
                    "tool_calls": true,
                    "vision": true
                }
            }
        }))
        .unwrap()
    }

    fn fixture_disabled_model() -> CopilotModel {
        serde_json::from_value(serde_json::json!({
            "id": "gpt-5.5",
            "name": "GPT-5.5",
            "model_picker_enabled": true,
            "policy": { "state": "disabled" },
            "capabilities": {
                "type": "chat",
                "limits": {
                    "max_context_window_tokens": 272000,
                    "max_output_tokens": 128000,
                    "max_prompt_tokens": 144000
                },
                "supports": { "tool_calls": true, "streaming": true }
            }
        }))
        .unwrap()
    }

    fn fixture_embedding_model() -> CopilotModel {
        serde_json::from_value(serde_json::json!({
            "id": "text-embedding-3-small",
            "name": "Embedding V3 small",
            "model_picker_enabled": false,
            "capabilities": {
                "type": "embeddings",
                "limits": { "max_inputs": 256 },
                "supports": { "dimensions": true }
            }
        }))
        .unwrap()
    }

    #[test]
    fn test_usable_chat_model() {
        let fixture = fixture_chat_model();
        let actual = fixture.is_usable();
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_policy_disabled_model_is_not_usable() {
        let fixture = fixture_disabled_model();
        let actual = fixture.is_usable();
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_embedding_model_is_not_usable() {
        let fixture = fixture_embedding_model();
        let actual = fixture.is_usable();
        let expected = false;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_model_without_policy_is_usable() {
        let fixture = serde_json::from_value::<CopilotModel>(serde_json::json!({
            "id": "gpt-4o",
            "name": "GPT-4o",
            "model_picker_enabled": false,
            "capabilities": {
                "type": "chat",
                "limits": {
                    "max_context_window_tokens": 128000,
                    "max_output_tokens": 16384,
                    "max_prompt_tokens": 64000
                },
                "supports": { "tool_calls": true, "parallel_tool_calls": true }
            }
        }))
        .unwrap();

        let actual = fixture.is_usable();
        let expected = true;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_conversion_to_domain_model() {
        let fixture = fixture_chat_model();

        let actual: forge_domain::Model = fixture.into();

        let expected = forge_domain::Model {
            id: ModelId::new("claude-sonnet-4.6"),
            name: Some("Claude Sonnet 4.6".to_string()),
            description: None,
            context_length: Some(264000),
            tools_supported: Some(true),
            supports_parallel_tool_calls: Some(true),
            supports_reasoning: Some(true),
            input_modalities: vec![
                forge_domain::InputModality::Text,
                forge_domain::InputModality::Image,
            ],
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_conversion_falls_back_to_max_prompt_tokens() {
        let fixture = serde_json::from_value::<CopilotModel>(serde_json::json!({
            "id": "some-model",
            "name": "Some Model",
            "capabilities": {
                "type": "chat",
                "limits": { "max_prompt_tokens": 100000, "max_output_tokens": 4096 },
                "supports": { "tool_calls": true }
            }
        }))
        .unwrap();

        let actual: forge_domain::Model = fixture.into();

        assert_eq!(actual.context_length, Some(100000));
        assert_eq!(actual.supports_reasoning, Some(false));
        assert_eq!(
            actual.input_modalities,
            vec![forge_domain::InputModality::Text]
        );
    }

    #[test]
    fn test_list_response_deserialization_ignores_unknown_fields() {
        let fixture = serde_json::json!({
            "data": [
                {
                    "id": "claude-sonnet-4.6",
                    "name": "Claude Sonnet 4.6",
                    "object": "model",
                    "model_picker_category": "versatile",
                    "warning_message": "some warning",
                    "capabilities": {
                        "type": "chat",
                        "object": "model_capabilities",
                        "tokenizer": "o200k_base",
                        "limits": { "max_prompt_tokens": 200000, "max_output_tokens": 64000 },
                        "supports": { "tool_calls": true }
                    }
                }
            ]
        });

        let actual = serde_json::from_value::<CopilotListModelResponse>(fixture).unwrap();

        assert_eq!(actual.data.len(), 1);
        assert_eq!(actual.data[0].id.as_str(), "claude-sonnet-4.6");
        assert_eq!(actual.data[0].is_usable(), true);
    }
}
