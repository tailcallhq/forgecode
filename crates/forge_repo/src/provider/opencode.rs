use std::sync::Arc;

use anyhow::Result;
use forge_app::domain::{
    ChatCompletionMessage, Context as ChatContext, InputModality, Model, ModelId, Provider,
    ProviderResponse, ResultStream,
};
use forge_app::{EnvironmentInfra, HttpInfra};
use forge_domain::ChatRepository;
use url::Url;

use crate::provider::anthropic::AnthropicResponseRepository;
use crate::provider::google::GoogleResponseRepository;
use crate::provider::openai::OpenAIResponseRepository;
use crate::provider::openai_responses::OpenAIResponsesResponseRepository;

/// OpenCode provider that routes to different backends based on model:
/// - Claude models (claude-*) -> Anthropic endpoint
/// - GPT-5 models (gpt-5*) -> OpenAIResponses endpoint
/// - Gemini models (gemini-*) -> Google endpoint
/// - Others (GLM, MiniMax, Kimi, etc.) -> OpenAI endpoint
///
/// Supports both OpenCode Zen and OpenCode Go by deriving endpoint URLs
/// from the provider's configured base URL rather than hardcoding them.
pub struct OpenCodeZenResponseRepository<F> {
    openai_repo: OpenAIResponseRepository<F>,
    codex_repo: OpenAIResponsesResponseRepository<F>,
    anthropic_repo: AnthropicResponseRepository<F>,
    google_repo: GoogleResponseRepository<F>,
}

impl<F: HttpInfra + EnvironmentInfra<Config = forge_config::ForgeConfig> + Sync>
    OpenCodeZenResponseRepository<F>
{
    pub fn new(infra: Arc<F>) -> Self {
        Self {
            openai_repo: OpenAIResponseRepository::new(infra.clone()),
            codex_repo: OpenAIResponsesResponseRepository::new(infra.clone()),
            anthropic_repo: AnthropicResponseRepository::new(infra.clone()),
            google_repo: GoogleResponseRepository::new(infra.clone()),
        }
    }

    /// Determines which backend to use based on the model ID
    fn get_backend(&self, model_id: &ModelId) -> OpenCodeBackend {
        let model_str = model_id.as_str();

        if model_str.starts_with("claude-") {
            OpenCodeBackend::Anthropic
        } else if model_str.starts_with("gpt-5") {
            OpenCodeBackend::OpenAIResponses
        } else if model_str.starts_with("gemini-") {
            OpenCodeBackend::Google
        } else {
            OpenCodeBackend::OpenAI
        }
    }

    /// Builds the appropriate provider for the given model.
    ///
    /// Derives the endpoint URL from the provider's configured base URL so that
    /// both OpenCode Zen and OpenCode Go (and any future variants) are routed
    /// to their correct endpoints.
    fn build_provider(&self, provider: &Provider<Url>, model_id: &ModelId) -> Provider<Url> {
        let backend = self.get_backend(model_id);
        let mut new_provider = provider.clone();
        let base = provider.url.as_str().trim_end_matches('/');

        match backend {
            OpenCodeBackend::Anthropic => {
                // Claude models use /v1/messages endpoint
                new_provider.url = Url::parse(&format!("{base}/v1/messages")).unwrap();
                new_provider.response = Some(ProviderResponse::Anthropic);
            }
            OpenCodeBackend::OpenAIResponses => {
                // GPT-5 models use /v1/responses endpoint
                new_provider.url = Url::parse(&format!("{base}/v1/responses")).unwrap();
                new_provider.response = Some(ProviderResponse::OpenAIResponses);
            }
            OpenCodeBackend::Google => {
                // Gemini models use model-specific endpoint
                new_provider.url = Url::parse(&format!("{base}/v1")).unwrap();
                new_provider.response = Some(ProviderResponse::Google);
            }
            OpenCodeBackend::OpenAI => {
                // Other models use /v1/chat/completions endpoint (default)
                new_provider.url = Url::parse(&format!("{base}/v1/chat/completions")).unwrap();
                new_provider.response = Some(ProviderResponse::OpenAI);
            }
        }

        new_provider
    }

    pub async fn chat(
        &self,
        model_id: &ModelId,
        context: ChatContext,
        provider: Provider<Url>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        let backend = self.get_backend(model_id);
        let adapted_provider = self.build_provider(&provider, model_id);

        match backend {
            OpenCodeBackend::Anthropic => {
                self.anthropic_repo
                    .chat(model_id, context, adapted_provider)
                    .await
            }
            OpenCodeBackend::OpenAIResponses => {
                self.codex_repo
                    .chat(model_id, context, adapted_provider)
                    .await
            }
            OpenCodeBackend::Google => {
                self.google_repo
                    .chat(model_id, context, adapted_provider)
                    .await
            }
            OpenCodeBackend::OpenAI => {
                self.openai_repo
                    .chat(model_id, context, adapted_provider)
                    .await
            }
        }
    }

    /// Returns the configured or remotely discovered models for the OpenCode provider.
    pub async fn models(&self, provider: Provider<Url>) -> Result<Vec<Model>> {
        match provider.models().cloned() {
            Some(forge_domain::ModelSource::Hardcoded(models)) => Ok(models),
            Some(forge_domain::ModelSource::Url(_)) => {
                let models = self.openai_repo.models(provider).await?;
                Ok(models.into_iter().map(enrich_remote_model).collect())
            }
            None => Ok(vec![]),
        }
    }
}

fn enrich_remote_model(mut model: Model) -> Model {
    let model_id = model.id.as_str().to_ascii_lowercase();

    model.tools_supported = Some(model.tools_supported.unwrap_or(true));
    model.supports_reasoning = Some(model.supports_reasoning.unwrap_or(true));
    model.supports_parallel_tool_calls = Some(model.supports_parallel_tool_calls.unwrap_or(
        !matches!(model_id.as_str(), "claude-3-5-haiku" | "claude-haiku-4-5"),
    ));

    if model.input_modalities == vec![InputModality::Text] && supports_image_input(&model_id) {
        model.input_modalities = vec![InputModality::Text, InputModality::Image];
    }

    model
}

fn supports_image_input(model_id: &str) -> bool {
    model_id.starts_with("claude-")
        || model_id.starts_with("gemini-")
        || model_id.starts_with("gpt-")
        || model_id.starts_with("minimax-")
        || model_id.starts_with("kimi-")
        || model_id == "qwen3.6-plus"
        || model_id.starts_with("mimo-v2-omni")
}

/// Backend type for OpenCode Zen routing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenCodeBackend {
    OpenAI,
    OpenAIResponses,
    Anthropic,
    Google,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use bytes::Bytes;
    use forge_app::domain::Environment;
    use forge_app::{EnvironmentInfra, HttpInfra};
    use pretty_assertions::assert_eq;
    use reqwest::header::HeaderMap;
    use reqwest_eventsource::EventSource;

    use super::*;
    use crate::provider::mock_server::MockServer;

    #[derive(Clone)]
    struct MockInfra {
        client: reqwest::Client,
    }

    impl MockInfra {
        fn new() -> Self {
            Self { client: reqwest::Client::new() }
        }
    }

    impl EnvironmentInfra for MockInfra {
        type Config = forge_config::ForgeConfig;

        fn get_env_var(&self, _key: &str) -> Option<String> {
            None
        }

        fn get_env_vars(&self) -> BTreeMap<String, String> {
            BTreeMap::new()
        }

        fn get_environment(&self) -> Environment {
            Environment {
                os: "test".to_string(),
                cwd: PathBuf::new(),
                home: None,
                shell: "bash".to_string(),
                base_path: PathBuf::new(),
            }
        }

        fn get_config(&self) -> anyhow::Result<Self::Config> {
            Ok(forge_config::ForgeConfig::default())
        }

        async fn update_environment(
            &self,
            _ops: Vec<forge_domain::ConfigOperation>,
        ) -> anyhow::Result<()> {
            unimplemented!()
        }
    }

    #[async_trait::async_trait]
    impl HttpInfra for MockInfra {
        async fn http_get(
            &self,
            url: &Url,
            headers: Option<HeaderMap>,
        ) -> anyhow::Result<reqwest::Response> {
            let mut request = self.client.get(url.clone());
            if let Some(headers) = headers {
                request = request.headers(headers);
            }
            Ok(request.send().await?)
        }

        async fn http_post(
            &self,
            _url: &Url,
            _headers: Option<HeaderMap>,
            _body: Bytes,
        ) -> anyhow::Result<reqwest::Response> {
            unimplemented!()
        }

        async fn http_delete(&self, _url: &Url) -> anyhow::Result<reqwest::Response> {
            unimplemented!()
        }

        async fn http_eventsource(
            &self,
            _url: &Url,
            _headers: Option<HeaderMap>,
            _body: Bytes,
        ) -> anyhow::Result<EventSource> {
            unimplemented!()
        }
    }

    fn create_provider(base_url: &str, models: forge_domain::ModelSource<Url>) -> Provider<Url> {
        Provider {
            id: forge_app::domain::ProviderId::OPENCODE_ZEN,
            provider_type: forge_domain::ProviderType::Llm,
            response: Some(forge_app::domain::ProviderResponse::OpenCode),
            url: Url::parse(base_url).unwrap(),
            credential: Some(forge_domain::AuthCredential {
                id: forge_app::domain::ProviderId::OPENCODE_ZEN,
                auth_details: forge_domain::AuthDetails::ApiKey(forge_domain::ApiKey::from(
                    "sk-test-key".to_string(),
                )),
                url_params: Default::default(),
            }),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            models: Some(models),
            custom_headers: None,
        }
    }

    fn fixture_model(id: &str) -> Model {
        Model {
            id: id.into(),
            name: None,
            description: None,
            context_length: None,
            tools_supported: None,
            supports_parallel_tool_calls: None,
            supports_reasoning: None,
            supported_reasoning_efforts: None,
            input_modalities: vec![InputModality::Text],
        }
    }

    /// Helper function to determine backend routing (mirrors get_backend logic)
    fn get_backend_for_test(model_id: &str) -> OpenCodeBackend {
        if model_id.starts_with("claude-") {
            OpenCodeBackend::Anthropic
        } else if model_id.starts_with("gpt-5") {
            OpenCodeBackend::OpenAIResponses
        } else if model_id.starts_with("gemini-") {
            OpenCodeBackend::Google
        } else {
            OpenCodeBackend::OpenAI
        }
    }

    #[test]
    fn test_model_routing() {
        // Test Claude models route to Anthropic
        assert_eq!(
            get_backend_for_test("claude-opus-4-6"),
            OpenCodeBackend::Anthropic
        );
        assert_eq!(
            get_backend_for_test("claude-sonnet-4-5"),
            OpenCodeBackend::Anthropic
        );
        assert_eq!(
            get_backend_for_test("claude-haiku-4-5"),
            OpenCodeBackend::Anthropic
        );

        // Test GPT-5 models route to OpenAIResponses
        assert_eq!(
            get_backend_for_test("gpt-5.4-pro"),
            OpenCodeBackend::OpenAIResponses
        );
        assert_eq!(
            get_backend_for_test("gpt-5"),
            OpenCodeBackend::OpenAIResponses
        );
        assert_eq!(
            get_backend_for_test("gpt-5.1-codex"),
            OpenCodeBackend::OpenAIResponses
        );

        // Test Gemini models route to Google
        assert_eq!(
            get_backend_for_test("gemini-3.1-pro"),
            OpenCodeBackend::Google
        );
        assert_eq!(
            get_backend_for_test("gemini-3-flash"),
            OpenCodeBackend::Google
        );

        // Test other models route to OpenAI
        assert_eq!(get_backend_for_test("glm-5"), OpenCodeBackend::OpenAI);
        assert_eq!(
            get_backend_for_test("minimax-m2.5"),
            OpenCodeBackend::OpenAI
        );
        assert_eq!(get_backend_for_test("kimi-k2.5"), OpenCodeBackend::OpenAI);
        assert_eq!(get_backend_for_test("big-pickle"), OpenCodeBackend::OpenAI);
    }

    #[tokio::test]
    async fn test_models_fetches_remote_openai_compatible_list() {
        let mut fixture = MockServer::new().await;
        let _mock = fixture
            .mock_models(
                serde_json::json!({
                    "data": [
                        {"id": "claude-opus-4-7", "object": "model", "created": 1},
                        {"id": "claude-haiku-4-5", "object": "model", "created": 1},
                        {"id": "glm-5.1", "object": "model", "created": 1}
                    ]
                }),
                200,
            )
            .await;
        let repository = OpenCodeZenResponseRepository::new(Arc::new(MockInfra::new()));
        let provider = create_provider(
            &format!("{}/zen", fixture.url()),
            forge_domain::ModelSource::Url(
                Url::parse(&format!("{}/models", fixture.url())).unwrap(),
            ),
        );

        let actual = repository.models(provider).await.unwrap();
        let expected = vec![
            fixture_model("claude-opus-4-7")
                .tools_supported(Some(true))
                .supports_parallel_tool_calls(Some(true))
                .supports_reasoning(Some(true))
                .input_modalities(vec![InputModality::Text, InputModality::Image]),
            fixture_model("claude-haiku-4-5")
                .tools_supported(Some(true))
                .supports_parallel_tool_calls(Some(false))
                .supports_reasoning(Some(true))
                .input_modalities(vec![InputModality::Text, InputModality::Image]),
            fixture_model("glm-5.1")
                .tools_supported(Some(true))
                .supports_parallel_tool_calls(Some(true))
                .supports_reasoning(Some(true))
                .input_modalities(vec![InputModality::Text]),
        ];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_models_returns_hardcoded_models_when_configured() {
        let repository = OpenCodeZenResponseRepository::new(Arc::new(MockInfra::new()));
        let fixture = vec![
            fixture_model("claude-opus-4-6")
                .tools_supported(Some(true))
                .supports_parallel_tool_calls(Some(true))
                .supports_reasoning(Some(true))
                .input_modalities(vec![InputModality::Text, InputModality::Image]),
        ];
        let provider = create_provider(
            "https://opencode.ai/zen",
            forge_domain::ModelSource::Hardcoded(fixture.clone()),
        );

        let actual = repository.models(provider).await.unwrap();
        let expected = fixture;

        assert_eq!(actual, expected);
    }
}
