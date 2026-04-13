use forge_domain::{DefaultTransformation, Provider, ProviderId, Transformer};
use url::Url;

use super::drop_tool_call::DropToolCalls;
use super::github_copilot_reasoning::GitHubCopilotReasoning;
use super::kimi_k2_reasoning::KimiK2Reasoning;
use super::make_cerebras_compat::MakeCerebrasCompat;
use super::make_openai_compat::MakeOpenAiCompat;
use super::make_xai_compat::MakeXaiCompat;
use super::minimax::SetMinimaxParams;
use super::normalize_tool_schema::{
    EnforceStrictResponseFormatSchema, EnforceStrictToolSchema, NormalizeToolSchema,
};
use super::set_cache::SetCache;
use super::set_reasoning_effort::SetReasoningEffort;
use super::strip_thought_signature::StripThoughtSignature;
use super::tool_choice::SetToolChoice;
use super::trim_tool_call_ids::TrimToolCallIds;
use super::when_model::when_model;
use super::zai_reasoning::SetZaiThinking;
use crate::dto::openai::{Request, ToolChoice};

/// Pipeline for transforming requests based on the provider type
pub struct ProviderPipeline<'a>(&'a Provider<Url>);

impl<'a> ProviderPipeline<'a> {
    /// Creates a new provider pipeline for the given provider
    pub fn new(provider: &'a Provider<Url>) -> Self {
        Self(provider)
    }
}

impl Transformer for ProviderPipeline<'_> {
    type Value = Request;

    fn transform(&mut self, request: Self::Value) -> Self::Value {
        // Only Anthropic and Gemini requires cache configuration to be set.
        // ref: https://openrouter.ai/docs/features/prompt-caching
        let provider = self.0;

        // Z.ai transformer must run before MakeOpenAiCompat which removes reasoning
        // field
        let zai_thinking = SetZaiThinking.when(move |_| is_zai_provider(provider));

        let or_transformers = DefaultTransformation::<Request>::new()
            .pipe(SetMinimaxParams.when(when_model("minimax")))
            .pipe(DropToolCalls.when(when_model("mistral")))
            .pipe(SetToolChoice::new(ToolChoice::Auto).when(when_model("gemini")))
            .pipe(SetCache.when(when_model("gemini|anthropic|minimax")))
            .when(move |_| supports_open_router_params(provider));

        // Strip thought signatures for all models except gemini-3
        let strip_thought_signature =
            StripThoughtSignature.when(move |req: &Request| !is_gemini3_model(req));

        let open_ai_compat = MakeOpenAiCompat.when(move |_| !supports_open_router_params(provider));

        let set_reasoning_effort = SetReasoningEffort.when(move |_| {
            provider.id == ProviderId::REQUESTY || provider.id == ProviderId::GITHUB_COPILOT
        });

        let github_copilot_reasoning =
            GitHubCopilotReasoning.when(move |_| provider.id == ProviderId::GITHUB_COPILOT);

        let kimi_k2_reasoning = KimiK2Reasoning.when(move |request: &Request| {
            provider.id == ProviderId::FIREWORKS_AI || when_model("kimi")(request)
        });

        let cerebras_compat = MakeCerebrasCompat.when(move |_| provider.id == ProviderId::CEREBRAS);

        let xai_compat = MakeXaiCompat.when(move |_| provider.id == ProviderId::XAI);

        let trim_tool_call_ids = TrimToolCallIds.when(move |_| provider.id == ProviderId::OPENAI);

        let strict_schema = EnforceStrictToolSchema
            .pipe(EnforceStrictResponseFormatSchema)
            .when(move |_| {
                provider.id == ProviderId::FIREWORKS_AI
                   
                    || provider.id == ProviderId::OPENCODE_ZEN
                    || provider.id == ProviderId::OPENCODE_GO
                    || provider.id == ProviderId::XAI
            });

        let mut combined = zai_thinking
            .pipe(or_transformers)
            .pipe(strip_thought_signature)
            .pipe(set_reasoning_effort)
            .pipe(open_ai_compat)
            .pipe(github_copilot_reasoning)
            .pipe(kimi_k2_reasoning)
            .pipe(cerebras_compat)
            .pipe(xai_compat)
            .pipe(trim_tool_call_ids)
            .pipe(strict_schema)
            .pipe(NormalizeToolSchema);
        combined.transform(request)
    }
}

/// Checks if provider is a z.ai provider (zai or zai_coding)
fn is_zai_provider(provider: &Provider<Url>) -> bool {
    provider.id == ProviderId::ZAI || provider.id == ProviderId::ZAI_CODING
}

/// Checks if the request model is a gemini-3 model (which supports thought
/// signatures)
fn is_gemini3_model(req: &Request) -> bool {
    req.model
        .as_ref()
        .map(|m| m.as_str().contains("gemini-3"))
        .unwrap_or(false)
}

/// function checks if provider supports open-router parameters.
fn supports_open_router_params(provider: &Provider<Url>) -> bool {
    provider.id == ProviderId::OPEN_ROUTER
        || provider.id == ProviderId::FORGE
        || provider.id == ProviderId::ZAI
        || provider.id == ProviderId::ZAI_CODING
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use forge_domain::ModelId;
    use url::Url;

    use super::*;
    use crate::domain::{ModelSource, ProviderResponse};

    // Test helper functions
    fn make_credential(provider_id: ProviderId, key: &str) -> Option<forge_domain::AuthCredential> {
        Some(forge_domain::AuthCredential {
            id: provider_id,
            auth_details: forge_domain::AuthDetails::ApiKey(forge_domain::ApiKey::from(
                key.to_string(),
            )),
            url_params: HashMap::new(),
        })
    }

    fn forge(key: &str) -> Provider<Url> {
        Provider {
            id: ProviderId::FORGE,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://antinomy.ai/api/v1/chat/completions").unwrap(),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            credential: make_credential(ProviderId::FORGE, key),
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("https://antinomy.ai/api/v1/models").unwrap(),
            )),
        }
    }

    fn zai(key: &str) -> Provider<Url> {
        Provider {
            id: ProviderId::ZAI,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.z.ai/api/paas/v4/chat/completions").unwrap(),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            credential: make_credential(ProviderId::ZAI, key),
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("https://api.z.ai/api/paas/v4/models").unwrap(),
            )),
        }
    }

    fn zai_coding(key: &str) -> Provider<Url> {
        Provider {
            id: ProviderId::ZAI_CODING,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.z.ai/api/coding/paas/v4/chat/completions").unwrap(),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            credential: make_credential(ProviderId::ZAI_CODING, key),
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("https://api.z.ai/api/paas/v4/models").unwrap(),
            )),
        }
    }

    fn openai(key: &str) -> Provider<Url> {
        Provider {
            id: ProviderId::OPENAI,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.openai.com/v1/chat/completions").unwrap(),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            credential: make_credential(ProviderId::OPENAI, key),
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("https://api.openai.com/v1/models").unwrap(),
            )),
        }
    }

    fn xai(key: &str) -> Provider<Url> {
        Provider {
            id: ProviderId::XAI,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.x.ai/v1/chat/completions").unwrap(),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            credential: make_credential(ProviderId::XAI, key),
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("https://api.x.ai/v1/models").unwrap(),
            )),
        }
    }

    fn requesty(key: &str) -> Provider<Url> {
        Provider {
            id: ProviderId::REQUESTY,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.requesty.ai/v1/chat/completions").unwrap(),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            credential: make_credential(ProviderId::REQUESTY, key),
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("https://api.requesty.ai/v1/models").unwrap(),
            )),
        }
    }

    fn open_router(key: &str) -> Provider<Url> {
        Provider {
            id: ProviderId::OPEN_ROUTER,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://openrouter.ai/api/v1/chat/completions").unwrap(),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            credential: make_credential(ProviderId::OPEN_ROUTER, key),
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("https://openrouter.ai/api/v1/models").unwrap(),
            )),
        }
    }

    fn anthropic(key: &str) -> Provider<Url> {
        Provider {
            id: ProviderId::ANTHROPIC,
            provider_type: Default::default(),
            response: Some(ProviderResponse::Anthropic),
            url: Url::parse("https://api.anthropic.com/v1/messages").unwrap(),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            credential: make_credential(ProviderId::ANTHROPIC, key),
            custom_headers: None,
            models: Some(ModelSource::Url(
                Url::parse("https://api.anthropic.com/v1/models").unwrap(),
            )),
        }
    }

    fn opencode_zen(key: &str) -> Provider<Url> {
        Provider {
            id: ProviderId::OPENCODE_ZEN,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://opencode.ai/zen/v1/chat/completions").unwrap(),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            credential: make_credential(ProviderId::OPENCODE_ZEN, key),
            custom_headers: None,
            models: Some(ModelSource::Hardcoded(vec![])),
        }
    }

    fn fireworks_ai(key: &str) -> Provider<Url> {
        Provider {
            id: ProviderId::FIREWORKS_AI,
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenAI),
            url: Url::parse("https://api.fireworks.ai/inference/v1/chat/completions").unwrap(),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            credential: make_credential(ProviderId::FIREWORKS_AI, key),
            custom_headers: None,
            models: Some(ModelSource::Hardcoded(vec![])),
        }
    }

    #[test]
    fn test_supports_open_router_params() {
        assert!(supports_open_router_params(&forge("forge")));
        assert!(supports_open_router_params(&open_router("open-router")));

        assert!(!supports_open_router_params(&requesty("requesty")));
        assert!(!supports_open_router_params(&openai("openai")));
        assert!(!supports_open_router_params(&xai("xai")));
        assert!(!supports_open_router_params(&anthropic("claude")));
    }

    #[test]
    fn test_is_zai_provider() {
        assert!(is_zai_provider(&zai("zai")));
        assert!(is_zai_provider(&zai_coding("zai-coding")));

        assert!(!is_zai_provider(&openai("openai")));
        assert!(!is_zai_provider(&anthropic("claude")));
        assert!(!is_zai_provider(&open_router("open-router")));
    }

    #[test]
    fn test_zai_provider_applies_thinking_transformation() {
        let provider = zai("zai");
        let fixture = Request::default().reasoning(forge_domain::ReasoningConfig {
            enabled: Some(true),
            effort: None,
            max_tokens: None,
            exclude: None,
        });

        let mut pipeline = ProviderPipeline::new(&provider);
        let actual = pipeline.transform(fixture);

        assert!(actual.thinking.is_some());
        assert_eq!(
            actual.thinking.unwrap().r#type,
            crate::dto::openai::ThinkingType::Enabled
        );
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_zai_coding_provider_applies_thinking_transformation() {
        let provider = zai_coding("zai-coding");
        let fixture = Request::default().reasoning(forge_domain::ReasoningConfig {
            enabled: Some(true),
            effort: None,
            max_tokens: None,
            exclude: None,
        });

        let mut pipeline = ProviderPipeline::new(&provider);
        let actual = pipeline.transform(fixture);

        assert!(actual.thinking.is_some());
        assert_eq!(
            actual.thinking.unwrap().r#type,
            crate::dto::openai::ThinkingType::Enabled
        );
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_non_zai_provider_doesnt_apply_thinking_transformation() {
        let provider = openai("openai");
        let fixture = Request::default().reasoning(forge_domain::ReasoningConfig {
            enabled: Some(true),
            effort: None,
            max_tokens: None,
            exclude: None,
        });

        let mut pipeline = ProviderPipeline::new(&provider);
        let actual = pipeline.transform(fixture);

        assert_eq!(actual.thinking, None);
        // OpenAI compat transformer removes reasoning field
        assert_eq!(actual.reasoning, None);
    }

    #[test]
    fn test_openai_provider_trims_tool_call_ids() {
        let provider = openai("openai");
        let long_id = "call_12345678901234567890123456789012345678901234567890";

        let fixture = Request::default().messages(vec![crate::dto::openai::Message {
            role: crate::dto::openai::Role::Tool,
            content: None,
            name: None,
            tool_call_id: Some(forge_domain::ToolCallId::new(long_id)),
            tool_calls: None,
            reasoning_details: None,
            reasoning_text: None,
            reasoning_opaque: None,
            reasoning_content: None,
            extra_content: None,
        }]);

        let mut pipeline = ProviderPipeline::new(&provider);
        let actual = pipeline.transform(fixture);

        let expected_id = "call_12345678901234567890123456789012345";
        assert_eq!(expected_id.len(), 40);

        let messages = actual.messages.unwrap();
        assert_eq!(
            messages[0].tool_call_id.as_ref().unwrap().as_str(),
            expected_id
        );
    }

    #[test]
    fn test_non_openai_provider_does_not_trim_tool_call_ids() {
        let provider = anthropic("claude");
        let long_id = "call_12345678901234567890123456789012345678901234567890";

        let fixture = Request::default().messages(vec![crate::dto::openai::Message {
            role: crate::dto::openai::Role::Tool,
            content: None,
            name: None,
            tool_call_id: Some(forge_domain::ToolCallId::new(long_id)),
            tool_calls: None,
            reasoning_details: None,
            reasoning_text: None,
            reasoning_opaque: None,
            reasoning_content: None,
            extra_content: None,
        }]);

        let mut pipeline = ProviderPipeline::new(&provider);
        let actual = pipeline.transform(fixture);

        // Anthropic provider should not trim tool call IDs
        let messages = actual.messages.unwrap();
        assert_eq!(messages[0].tool_call_id.as_ref().unwrap().as_str(), long_id);
    }

    #[test]
    fn test_gemini3_model_preserves_thought_signature() {
        use crate::dto::openai::{ExtraContent, GoogleMetadata, Message, MessageContent, Role};

        let provider = open_router("open-router");
        let fixture = Request::default()
            .model(ModelId::new("google/gemini-3-pro-preview"))
            .messages(vec![Message {
                role: Role::Assistant,
                content: Some(MessageContent::Text("Hello".to_string())),
                name: None,
                tool_call_id: None,
                tool_calls: None,
                reasoning_details: None,
                reasoning_text: None,
                reasoning_opaque: None,
                reasoning_content: None,
                extra_content: Some(ExtraContent {
                    google: Some(GoogleMetadata { thought_signature: Some("sig123".to_string()) }),
                }),
            }]);

        let mut pipeline = ProviderPipeline::new(&provider);
        let actual = pipeline.transform(fixture);

        // Thought signature should be preserved for gemini-3 models
        let messages = actual.messages.unwrap();
        assert!(messages[0].extra_content.is_some());
        assert_eq!(
            messages[0]
                .extra_content
                .as_ref()
                .unwrap()
                .google
                .as_ref()
                .unwrap()
                .thought_signature,
            Some("sig123".to_string())
        );
    }

    #[test]
    fn test_non_gemini3_model_strips_thought_signature() {
        use crate::dto::openai::{ExtraContent, GoogleMetadata, Message, MessageContent, Role};

        let provider = open_router("open-router");
        let fixture = Request::default()
            .model(ModelId::new("anthropic/claude-sonnet-4"))
            .messages(vec![Message {
                role: Role::Assistant,
                content: Some(MessageContent::Text("Hello".to_string())),
                name: None,
                tool_call_id: None,
                tool_calls: None,
                reasoning_details: None,
                reasoning_text: None,
                reasoning_opaque: None,
                reasoning_content: None,
                extra_content: Some(ExtraContent {
                    google: Some(GoogleMetadata { thought_signature: Some("sig123".to_string()) }),
                }),
            }]);

        let mut pipeline = ProviderPipeline::new(&provider);
        let actual = pipeline.transform(fixture);

        // Thought signature should be stripped for non-gemini-3 models
        let messages = actual.messages.unwrap();
        assert!(messages[0].extra_content.is_none());
    }

    #[test]
    fn test_minimax_model_applies_cache_via_open_router() {
        use crate::dto::openai::{Message, MessageContent, Role};

        let provider = open_router("open-router");
        let fixture = Request::default()
            .model(ModelId::new("minimax/minimax-m2"))
            .messages(vec![
                Message {
                    role: Role::User,
                    content: Some(MessageContent::Text("Hello".to_string())),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                    reasoning_details: None,
                    reasoning_text: None,
                    reasoning_opaque: None,
                    reasoning_content: None,
                    extra_content: None,
                },
                Message {
                    role: Role::Assistant,
                    content: Some(MessageContent::Text("Hi there".to_string())),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                    reasoning_details: None,
                    reasoning_text: None,
                    reasoning_opaque: None,
                    reasoning_content: None,
                    extra_content: None,
                },
            ]);

        let mut pipeline = ProviderPipeline::new(&provider);
        let actual = pipeline.transform(fixture);

        // Cache should be applied: first and last messages cached
        let messages = actual.messages.unwrap();
        assert!(
            messages
                .first()
                .unwrap()
                .content
                .as_ref()
                .unwrap()
                .is_cached()
        );
        assert!(
            messages
                .last()
                .unwrap()
                .content
                .as_ref()
                .unwrap()
                .is_cached()
        );
    }

    #[test]
    fn test_non_minimax_model_does_not_apply_cache_via_open_router() {
        use crate::dto::openai::{Message, MessageContent, Role};

        let provider = open_router("open-router");
        let fixture = Request::default()
            .model(ModelId::new("openai/gpt-4o"))
            .messages(vec![
                Message {
                    role: Role::User,
                    content: Some(MessageContent::Text("Hello".to_string())),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                    reasoning_details: None,
                    reasoning_text: None,
                    reasoning_opaque: None,
                    reasoning_content: None,
                    extra_content: None,
                },
                Message {
                    role: Role::Assistant,
                    content: Some(MessageContent::Text("Hi there".to_string())),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                    reasoning_details: None,
                    reasoning_text: None,
                    reasoning_opaque: None,
                    reasoning_content: None,
                    extra_content: None,
                },
            ]);

        let mut pipeline = ProviderPipeline::new(&provider);
        let actual = pipeline.transform(fixture);

        // Cache should NOT be applied for non-minimax/gemini/anthropic models
        let messages = actual.messages.unwrap();
        assert!(
            !messages
                .first()
                .unwrap()
                .content
                .as_ref()
                .unwrap()
                .is_cached()
        );
        assert!(
            !messages
                .last()
                .unwrap()
                .content
                .as_ref()
                .unwrap()
                .is_cached()
        );
    }

    #[test]
    fn test_gemini2_model_strips_thought_signature() {
        use crate::dto::openai::{ExtraContent, GoogleMetadata, Message, MessageContent, Role};

        let provider = open_router("open-router");
        let fixture = Request::default()
            .model(ModelId::new("google/gemini-2.5-pro"))
            .messages(vec![Message {
                role: Role::Assistant,
                content: Some(MessageContent::Text("Hello".to_string())),
                name: None,
                tool_call_id: None,
                tool_calls: None,
                reasoning_details: None,
                reasoning_text: None,
                reasoning_opaque: None,
                reasoning_content: None,
                extra_content: Some(ExtraContent {
                    google: Some(GoogleMetadata { thought_signature: Some("sig123".to_string()) }),
                }),
            }]);

        let mut pipeline = ProviderPipeline::new(&provider);
        let actual = pipeline.transform(fixture);

        // Thought signature should be stripped for gemini-2 models (not gemini-3)
        let messages = actual.messages.unwrap();
        assert!(messages[0].extra_content.is_none());
    }

    #[test]
    fn test_opencode_zen_provider_enforces_strict_tool_schema() {
        let provider = opencode_zen("opencode-zen");
        let fixture = Request::default().tools(vec![crate::dto::openai::Tool {
            r#type: crate::dto::openai::FunctionType,
            function: crate::dto::openai::FunctionDescription {
                name: "fs_search".to_string(),
                description: Some("Search files".to_string()),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "output_mode": {
                            "description": "Output mode",
                            "nullable": true,
                            "type": "string",
                            "enum": ["content", "files_with_matches", "count", null]
                        }
                    }
                }),
            },
        }]);

        let mut pipeline = ProviderPipeline::new(&provider);
        let actual = pipeline.transform(fixture);

        let expected = serde_json::json!({
            "type": "object",
            "properties": {
                "output_mode": {
                    "description": "Output mode",
                    "anyOf": [
                        {"type": "string", "enum": ["content", "files_with_matches", "count"]},
                        {"type": "null"}
                    ]
                }
            },
            "additionalProperties": false,
            "required": ["output_mode"]
        });

        assert_eq!(actual.tools.unwrap()[0].function.parameters, expected);
    }

    #[test]
    fn test_fireworks_provider_enforces_strict_tool_and_response_format_schemas() {
        let provider = fireworks_ai("fireworks-ai");
        let fixture = Request::default()
            .tools(vec![crate::dto::openai::Tool {
                r#type: crate::dto::openai::FunctionType,
                function: crate::dto::openai::FunctionDescription {
                    name: "fs_search".to_string(),
                    description: Some("Search files".to_string()),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "output_mode": {
                                "description": "Output mode",
                                "nullable": true,
                                "type": "string",
                                "enum": ["content", "files_with_matches", "count", null]
                            }
                        }
                    }),
                },
            }])
            .response_format(crate::dto::openai::ResponseFormat::JsonSchema {
                name: "test_response".to_string(),
                schema: Box::new(
                    schemars::Schema::try_from(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "result": {
                                "description": "Result",
                                "nullable": true,
                                "type": "string",
                                "enum": ["done", null]
                            }
                        }
                    }))
                    .unwrap(),
                ),
            });

        let mut pipeline = ProviderPipeline::new(&provider);
        let actual = pipeline.transform(fixture);

        let expected_tool_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "output_mode": {
                    "description": "Output mode",
                    "anyOf": [
                        {"type": "string", "enum": ["content", "files_with_matches", "count"]},
                        {"type": "null"}
                    ]
                }
            },
            "additionalProperties": false,
            "required": ["output_mode"]
        });
        assert_eq!(
            actual.tools.as_ref().unwrap()[0].function.parameters,
            expected_tool_schema
        );

        let actual_response_schema = match actual.response_format {
            Some(crate::dto::openai::ResponseFormat::JsonSchema { schema, .. }) => {
                serde_json::to_value(schema).unwrap()
            }
            Some(crate::dto::openai::ResponseFormat::Text) => {
                panic!("Expected json_schema response format")
            }
            None => panic!("Expected response format to be preserved"),
        };
        let expected_response_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "result": {
                    "description": "Result",
                    "anyOf": [
                        {"type": "string", "enum": ["done"]},
                        {"type": "null"}
                    ]
                }
            },
            "additionalProperties": false,
            "required": ["result"]
        });

        assert_eq!(actual_response_schema, expected_response_schema);
    }

    #[test]
    fn test_fireworks_provider_converts_reasoning_details_to_reasoning_content() {
        let provider = fireworks_ai("fireworks-ai");
        let fixture = Request::default().messages(vec![crate::dto::openai::Message {
            role: crate::dto::openai::Role::Assistant,
            content: Some(crate::dto::openai::MessageContent::Text("test".to_string())),
            name: None,
            tool_call_id: None,
            tool_calls: None,
            reasoning_details: Some(vec![crate::dto::openai::ReasoningDetail {
                r#type: "reasoning.text".to_string(),
                text: Some("thinking...".to_string()),
                signature: None,
                data: None,
                id: None,
                format: None,
                index: None,
            }]),
            reasoning_text: None,
            reasoning_opaque: None,
            reasoning_content: None,
            extra_content: None,
        }]);

        let mut pipeline = ProviderPipeline::new(&provider);
        let actual = pipeline.transform(fixture);

        let message = actual.messages.unwrap().into_iter().next().unwrap();
        assert_eq!(message.reasoning_content, Some("thinking...".to_string()));
        assert!(message.reasoning_details.is_none());
    }

    #[test]
    fn test_openai_provider_does_not_enforce_strict_tool_schema() {
        let provider = openai("openai");
        let fixture = Request::default().tools(vec![crate::dto::openai::Tool {
            r#type: crate::dto::openai::FunctionType,
            function: crate::dto::openai::FunctionDescription {
                name: "fs_search".to_string(),
                description: Some("Search files".to_string()),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "output_mode": {
                            "description": "Output mode",
                            "nullable": true,
                            "type": "string",
                            "enum": ["content", "files_with_matches", "count", null]
                        }
                    }
                }),
            },
        }]);

        let mut pipeline = ProviderPipeline::new(&provider);
        let actual = pipeline.transform(fixture);

        let expected = serde_json::json!({
            "type": "object",
            "properties": {
                "output_mode": {
                    "description": "Output mode",
                    "nullable": true,
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count", null]
                }
            }
        });

        assert_eq!(actual.tools.unwrap()[0].function.parameters, expected);
    }
}
