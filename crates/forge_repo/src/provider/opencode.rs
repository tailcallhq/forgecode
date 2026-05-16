use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use forge_app::domain::{
    ChatCompletionMessage, Context as ChatContext, Model, ModelId, Provider, ProviderResponse,
    ResultStream,
};
use forge_app::{EnvironmentInfra, HttpInfra};
use forge_domain::{ChatRepository, InputModality};
use serde::Deserialize;
use url::Url;

use crate::provider::anthropic::AnthropicResponseRepository;
use crate::provider::google::GoogleResponseRepository;
use crate::provider::openai::OpenAIResponseRepository;
use crate::provider::openai_responses::OpenAIResponsesResponseRepository;

const MODELS_DEV_API_URL: &str = "https://models.dev/api.json";

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
    infra: Arc<F>,
}

/// Response from models.dev API for a single model.
#[derive(Debug, Deserialize)]
struct ModelsDevModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    family: Option<String>,
    #[serde(default)]
    reasoning: bool,
    #[serde(default)]
    tool_call: bool,
    #[serde(default)]
    knowledge: Option<String>,
    #[serde(default)]
    modalities: Option<ModelsDevModalities>,
    #[serde(default)]
    limit: Option<ModelsDevLimit>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevModalities {
    #[serde(default)]
    input: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevLimit {
    #[serde(default)]
    context: Option<u64>,
}

/// Response from models.dev API.
#[derive(Debug, Deserialize)]
struct ModelsDevResponse {
    #[serde(default)]
    models: HashMap<String, ModelsDevModel>,
}

/// Maps a Forge provider ID to the corresponding models.dev provider key.
fn models_dev_key(provider_id: &forge_domain::ProviderId) -> Option<&'static str> {
    if *provider_id == forge_domain::ProviderId::OPENCODE_ZEN {
        Some("opencode")
    } else if *provider_id == forge_domain::ProviderId::OPENCODE_GO {
        Some("opencode-go")
    } else {
        None
    }
}

/// Converts a models.dev model entry into a Forge domain model.
fn into_domain_model(model: ModelsDevModel) -> Model {
    let input_modalities = model
        .modalities
        .map(|m| {
            m.input
                .iter()
                .filter_map(|s| match s.as_str() {
                    "text" => Some(InputModality::Text),
                    "image" => Some(InputModality::Image),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![InputModality::Text]);

    let description = model.family.map(|family| {
        let mut parts = vec![family];
        if let Some(knowledge) = model.knowledge {
            parts.push(format!("knowledge cutoff: {knowledge}"));
        }
        parts.join(", ")
    });

    Model {
        id: ModelId::new(model.id),
        name: model.name,
        description,
        context_length: model.limit.and_then(|l| l.context),
        tools_supported: Some(model.tool_call),
        supports_parallel_tool_calls: None,
        supports_reasoning: Some(model.reasoning),
        input_modalities,
    }
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
            infra,
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

    pub async fn models(&self, provider: Provider<Url>) -> Result<Vec<Model>> {
        let hardcoded_models = match provider.models() {
            Some(forge_domain::ModelSource::Hardcoded(models)) => models.clone(),
            _ => vec![],
        };

        // Fetch live models from models.dev — the same source opencode CLI uses.
        let fetched_models = match self.fetch_models_dev(&provider).await {
            Ok(models) => models,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    provider = %provider.id,
                    "Failed to fetch dynamic models from models.dev, falling back to hardcoded"
                );
                return Ok(hardcoded_models);
            }
        };

        // Build a lookup map from hardcoded models for metadata enrichment.
        let hardcoded_map: HashMap<_, _> = hardcoded_models
            .into_iter()
            .map(|m| (m.id.clone(), m))
            .collect();

        // Merge: use hardcoded metadata when available, otherwise use fetched data.
        let merged = fetched_models
            .into_iter()
            .map(|f| hardcoded_map.get(&f.id).cloned().unwrap_or(f))
            .collect();

        Ok(merged)
    }

    /// Fetches the up-to-date model list from models.dev for the given
    /// provider.
    async fn fetch_models_dev(&self, provider: &Provider<Url>) -> Result<Vec<Model>> {
        let provider_key = models_dev_key(&provider.id)
            .ok_or_else(|| anyhow::anyhow!("Unknown provider for models.dev: {}", provider.id))?;

        let url = Url::parse(MODELS_DEV_API_URL)?;
        let response = self.infra.http_get(&url, None).await?;
        let body = response.text().await?;
        let mut api: HashMap<String, ModelsDevResponse> = serde_json::from_str(&body)?;

        let provider_data = api.remove(provider_key).ok_or_else(|| {
            anyhow::anyhow!("Provider {} not found in models.dev response", provider_key)
        })?;

        let models = provider_data
            .models
            .into_values()
            .map(into_domain_model)
            .collect();

        Ok(models)
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
    use pretty_assertions::assert_eq;

    use super::*;

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
