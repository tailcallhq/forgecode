use std::sync::Arc;

use anyhow::Result;
use forge_app::domain::{
    ChatCompletionMessage, Context as ChatContext, Model, ModelId, Provider, ProviderResponse,
    ResultStream,
};
use forge_app::{EnvironmentInfra, HttpInfra};
use forge_domain::{ChatRepository, ConversationId};
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
    fn build_provider(
        &self,
        provider: &Provider<Url>,
        model_id: &ModelId,
        conversation_id: ConversationId,
    ) -> Provider<Url> {
        let backend = self.get_backend(model_id);
        let mut new_provider = provider.clone();
        // This clone belongs to one chat request, never to the shared provider
        // config. Keep session metadata outside the body
        // transformations used by each adapter.
        let headers = new_provider.custom_headers.get_or_insert_default();
        headers.retain(|name, _| !name.eq_ignore_ascii_case("x-opencode-session"));
        headers.insert(
            "x-opencode-session".to_string(),
            conversation_id.to_string(),
        );
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
        // Standalone requests without a conversation still need a session, but
        // must not share a provider-wide ID with unrelated requests.
        let conversation_id = context
            .conversation_id
            .unwrap_or_else(ConversationId::generate);
        let adapted_provider = self.build_provider(&provider, model_id, conversation_id);

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

    pub async fn models(&self, provider: Provider<Url>) -> Result<Vec<Model>> {
        // For OpenCode Zen, we use hardcoded models from the provider config
        // The models are already loaded from provider.json
        if let Some(models) = provider.models() {
            match models {
                forge_domain::ModelSource::Hardcoded(models) => Ok(models.clone()),
                forge_domain::ModelSource::Url(_) => {
                    // Should not happen for OpenCode Zen as we hardcode models
                    Ok(vec![])
                }
            }
        } else {
            Ok(vec![])
        }
    }
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
    use std::collections::{BTreeMap, HashMap};
    use std::sync::Mutex;

    use bytes::Bytes;
    use forge_domain::{AuthCredential, AuthDetails, ContextMessage, ProviderId};
    use forge_eventsource::EventSource;
    use pretty_assertions::assert_eq;
    use reqwest::header::HeaderMap;

    use super::*;

    #[derive(Default)]
    struct RecordingInfra {
        requests: Mutex<Vec<(Url, HeaderMap, Bytes)>>,
    }

    impl EnvironmentInfra for RecordingInfra {
        type Config = forge_config::ForgeConfig;

        fn get_config(&self) -> anyhow::Result<Self::Config> {
            Ok(Self::Config::default())
        }

        fn get_environment(&self) -> forge_domain::Environment {
            use fake::{Fake, Faker};
            Faker.fake()
        }

        fn get_env_var(&self, _: &str) -> Option<String> {
            None
        }

        fn get_env_vars(&self) -> BTreeMap<String, String> {
            BTreeMap::new()
        }

        async fn update_environment(&self, _: Vec<forge_domain::ConfigOperation>) -> Result<()> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl HttpInfra for RecordingInfra {
        async fn http_delete(&self, _: &Url) -> Result<reqwest::Response> {
            anyhow::bail!("Unexpected DELETE")
        }

        async fn http_get(&self, _: &Url, _: Option<HeaderMap>) -> Result<reqwest::Response> {
            anyhow::bail!("Unexpected GET")
        }

        async fn http_post(
            &self,
            url: &Url,
            headers: Option<HeaderMap>,
            body: Bytes,
        ) -> Result<reqwest::Response> {
            self.requests
                .lock()
                .unwrap()
                .push((url.clone(), headers.unwrap(), body));
            anyhow::bail!("Recorded request")
        }

        async fn http_eventsource(
            &self,
            url: &Url,
            headers: Option<HeaderMap>,
            body: Bytes,
        ) -> Result<EventSource> {
            self.requests
                .lock()
                .unwrap()
                .push((url.clone(), headers.unwrap(), body));
            anyhow::bail!("Recorded request")
        }
    }

    fn fixture_provider(id: ProviderId) -> anyhow::Result<Provider<Url>> {
        Ok(Provider {
            id: id.clone(),
            provider_type: Default::default(),
            response: Some(ProviderResponse::OpenCode),
            url: Url::parse("https://opencode.ai/zen/go")?,
            models: Some(forge_domain::ModelSource::Hardcoded(vec![])),
            auth_methods: vec![forge_domain::AuthMethod::ApiKey],
            url_params: vec![],
            credential: Some(AuthCredential {
                id,
                auth_details: AuthDetails::ApiKey("fixture-key".to_string().into()),
                url_params: HashMap::new(),
            }),
            custom_headers: Some(HashMap::from([
                (
                    "X-OpenCode-Session".to_string(),
                    "stale-static-session".to_string(),
                ),
                ("x-custom".to_string(), "preserved".to_string()),
            ])),
        })
    }

    #[tokio::test]
    async fn test_session_reaches_every_opencode_adapter() {
        for provider_id in [ProviderId::OPENCODE_GO, ProviderId::OPENCODE_ZEN] {
            let fixture = fixture_provider(provider_id).unwrap();
            let original = fixture.clone();
            let infra = Arc::new(RecordingInfra::default());
            let repo = OpenCodeZenResponseRepository::new(infra.clone());
            let conversation_id = ConversationId::generate();
            let other_id = ConversationId::generate();
            let context = ChatContext::default()
                .conversation_id(conversation_id)
                .add_message(ContextMessage::user("hello", None));
            let models = [
                ("deepseek-v4-flash", "/zen/go/v1/chat/completions"),
                ("gpt-5", "/zen/go/v1/responses"),
                ("claude-sonnet-4-5", "/zen/go/v1/messages"),
                (
                    "gemini-3-flash",
                    "/zen/go/v1/models/gemini-3-flash:streamGenerateContent",
                ),
            ];

            // Repeated turns/retries and resumed contexts use the same ID; an
            // interleaved conversation must not overwrite that session.
            for id in [conversation_id, other_id, conversation_id] {
                for (model, _) in models {
                    let result = repo
                        .chat(
                            &ModelId::new(model),
                            context.clone().conversation_id(id),
                            fixture.clone(),
                        )
                        .await;
                    assert!(result.is_err());
                }
            }
            let requests = infra.requests.lock().unwrap();
            let actual = requests
                .iter()
                .map(|(url, headers, body)| {
                    let json: serde_json::Value = serde_json::from_slice(body).unwrap();
                    assert!(json.get("session_id").is_none());
                    (
                        url.path().to_string(),
                        headers
                            .get("x-opencode-session")
                            .map(|value| value.to_str().unwrap().to_string()),
                        headers
                            .get("x-custom")
                            .map(|value| value.to_str().unwrap().to_string()),
                        headers.get_all("x-opencode-session").iter().count(),
                    )
                })
                .collect::<Vec<_>>();
            let expected = [conversation_id, other_id, conversation_id]
                .into_iter()
                .flat_map(|id| {
                    models.map(|(_, path)| {
                        (
                            path.to_string(),
                            Some(id.to_string()),
                            Some("preserved".to_string()),
                            1,
                        )
                    })
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
            assert_eq!(fixture, original);
        }
    }

    #[tokio::test]
    async fn test_standalone_opencode_requests_have_independent_sessions() {
        let fixture = fixture_provider(ProviderId::OPENCODE_GO).unwrap();
        let infra = Arc::new(RecordingInfra::default());
        let repo = OpenCodeZenResponseRepository::new(infra.clone());
        for _ in 0..2 {
            let result = repo
                .chat(
                    &ModelId::new("deepseek-v4-flash"),
                    ChatContext::default(),
                    fixture.clone(),
                )
                .await;
            assert!(result.is_err());
        }
        let requests = infra.requests.lock().unwrap();
        let actual = requests
            .iter()
            .map(|(_, headers, _)| {
                ConversationId::parse(headers["x-opencode-session"].to_str().unwrap()).unwrap()
            })
            .collect::<std::collections::HashSet<_>>()
            .len();
        let expected = 2;
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_other_providers_do_not_receive_opencode_session() {
        let mut fixture = fixture_provider(ProviderId::OPENAI).unwrap();
        fixture.custom_headers = None;
        let infra = Arc::new(RecordingInfra::default());
        let context = ChatContext::default().conversation_id(ConversationId::generate());
        let model = ModelId::new("fixture-model");

        let result = OpenAIResponseRepository::new(infra.clone())
            .chat(&model, context.clone(), fixture.clone())
            .await;
        assert!(result.is_err());
        let result = OpenAIResponsesResponseRepository::new(infra.clone())
            .chat(&model, context.clone(), fixture.clone())
            .await;
        assert!(result.is_err());
        let result = AnthropicResponseRepository::new(infra.clone())
            .chat(&model, context.clone(), fixture.clone())
            .await;
        assert!(result.is_err());
        let result = GoogleResponseRepository::new(infra.clone())
            .chat(&model, context, fixture)
            .await;
        assert!(result.is_err());
        let actual = infra
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|(_, headers, _)| headers.contains_key("x-opencode-session"))
            .collect::<Vec<_>>();
        let expected = vec![false, false, false, false];
        assert_eq!(actual, expected);
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
}
