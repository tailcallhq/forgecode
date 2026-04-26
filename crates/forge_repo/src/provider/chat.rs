use std::sync::Arc;

use forge_app::domain::{
    ChatCompletionMessage, Context, Model, ModelId, ProviderResponse, ResultStream,
};
use forge_app::{EnvironmentInfra, HttpInfra};
use forge_domain::{ChatRepository, Provider, ProviderId};
use forge_infra::CacacheStorage;
use tokio::task::AbortHandle;
use url::Url;

use crate::provider::anthropic::AnthropicResponseRepository;
use crate::provider::bedrock::BedrockResponseRepository;
use crate::provider::google::GoogleResponseRepository;
use crate::provider::openai::OpenAIResponseRepository;
use crate::provider::openai_responses::OpenAIResponsesResponseRepository;
use crate::provider::opencode::OpenCodeZenResponseRepository;

/// Repository responsible for routing chat requests to the appropriate provider
/// implementation based on the provider's response type.
pub struct ForgeChatRepository<F> {
    router: Arc<ProviderRouter<F>>,
    model_cache: Arc<CacacheStorage>,
    bg_refresh: BgRefresh,
}

impl<F: EnvironmentInfra<Config = forge_config::ForgeConfig> + HttpInfra> ForgeChatRepository<F> {
    /// Creates a new ForgeChatRepository with the given infrastructure.
    ///
    /// # Arguments
    ///
    /// * `infra` - Infrastructure providing environment and HTTP capabilities
    pub fn new(infra: Arc<F>) -> Self {
        let env = infra.get_environment();
        let config = infra.get_config().unwrap_or_default();
        let model_cache_ttl_secs = config.model_cache_ttl_secs;

        let openai_repo = OpenAIResponseRepository::new(infra.clone());
        let codex_repo = OpenAIResponsesResponseRepository::new(infra.clone());
        let anthropic_repo = AnthropicResponseRepository::new(infra.clone());
        let bedrock_repo =
            BedrockResponseRepository::new(Arc::new(config.retry.unwrap_or_default()));
        let google_repo = GoogleResponseRepository::new(infra.clone());
        let opencode_zen_repo = OpenCodeZenResponseRepository::new(infra.clone());

        let model_cache = Arc::new(CacacheStorage::new(
            env.cache_dir().join("model_cache"),
            Some(model_cache_ttl_secs as u128),
        ));

        Self {
            router: Arc::new(ProviderRouter {
                openai_repo,
                codex_repo,
                anthropic_repo,
                bedrock_repo,
                google_repo,
                opencode_zen_repo,
            }),
            model_cache,
            bg_refresh: BgRefresh::default(),
        }
    }
}

#[async_trait::async_trait]
impl<F: EnvironmentInfra<Config = forge_config::ForgeConfig> + HttpInfra + Sync> ChatRepository
    for ForgeChatRepository<F>
{
    async fn chat(
        &self,
        model_id: &ModelId,
        context: Context,
        provider: Provider<Url>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        self.router.chat(model_id, context, provider).await
    }

    async fn models(&self, provider: Provider<Url>) -> anyhow::Result<Vec<Model>> {
        use forge_app::KVStore;

        let cache_key = format!("models:{}", provider.id);

        if let Ok(Some(cached)) = self
            .model_cache
            .cache_get::<_, Vec<Model>>(&cache_key)
            .await
        {
            tracing::debug!(provider_id = %provider.id, "returning cached models; refreshing in background");

            // Spawn a background task to refresh the disk cache. The abort
            // handle is stored so the task is cancelled if the service is dropped.
            let cache = self.model_cache.clone();
            let router = self.router.clone();
            let key = cache_key;
            let handle = tokio::spawn(async move {
                match router.models(provider).await {
                    Ok(models) => {
                        if let Err(err) = cache.cache_set(&key, &models).await {
                            tracing::warn!(error = %err, "background refresh: failed to cache model list");
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "background refresh: failed to fetch models");
                    }
                }
            });
            self.bg_refresh.register(handle.abort_handle());

            return Ok(cached);
        }

        let models = self.router.models(provider).await?;

        if let Err(err) = self.model_cache.cache_set(&cache_key, &models).await {
            tracing::warn!(error = %err, "failed to cache model list");
        }

        Ok(models)
    }
}

/// Routes chat and model requests to the correct provider backend.
struct ProviderRouter<F> {
    openai_repo: OpenAIResponseRepository<F>,
    codex_repo: OpenAIResponsesResponseRepository<F>,
    anthropic_repo: AnthropicResponseRepository<F>,
    bedrock_repo: BedrockResponseRepository,
    google_repo: GoogleResponseRepository<F>,
    opencode_zen_repo: OpenCodeZenResponseRepository<F>,
}

impl<F: HttpInfra + EnvironmentInfra<Config = forge_config::ForgeConfig> + Sync> ProviderRouter<F> {
    async fn chat(
        &self,
        model_id: &ModelId,
        context: Context,
        provider: Provider<Url>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        match provider.response {
            Some(ProviderResponse::OpenAI) => {
                // Check if model is a Codex model
                if model_id.as_str().contains("gpt-5")
                    && (provider.id == ProviderId::OPENAI
                        || provider.id == ProviderId::GITHUB_COPILOT
                        || provider.id == ProviderId::CODEX)
                {
                    self.codex_repo.chat(model_id, context, provider).await
                } else if provider.id == ProviderId::CODEX {
                    // All Codex provider models use the Responses API
                    self.codex_repo.chat(model_id, context, provider).await
                } else {
                    self.openai_repo.chat(model_id, context, provider).await
                }
            }
            Some(ProviderResponse::OpenAIResponses) => {
                self.codex_repo.chat(model_id, context, provider).await
            }
            Some(ProviderResponse::Anthropic) => {
                self.anthropic_repo.chat(model_id, context, provider).await
            }
            Some(ProviderResponse::Bedrock) => {
                self.bedrock_repo.chat(model_id, context, provider).await
            }
            Some(ProviderResponse::Google) => {
                self.google_repo.chat(model_id, context, provider).await
            }
            Some(ProviderResponse::OpenCode) => {
                self.opencode_zen_repo
                    .chat(model_id, context, provider)
                    .await
            }
            None => Err(anyhow::anyhow!(
                "Provider response type not configured for provider: {}",
                provider.id
            )),
        }
    }

    async fn models(&self, provider: Provider<Url>) -> anyhow::Result<Vec<Model>> {
        match provider.response {
            Some(ProviderResponse::OpenAI) => self.openai_repo.models(provider).await,
            Some(ProviderResponse::OpenAIResponses) => self.codex_repo.models(provider).await,
            Some(ProviderResponse::Anthropic) => self.anthropic_repo.models(provider).await,
            Some(ProviderResponse::Bedrock) => self.bedrock_repo.models(provider).await,
            Some(ProviderResponse::Google) => self.google_repo.models(provider).await,
            Some(ProviderResponse::OpenCode) => self.opencode_zen_repo.models(provider).await,
            None => Err(anyhow::anyhow!(
                "Provider response type not configured for provider: {}",
                provider.id
            )),
        }
    }
}

/// Tracks abort handles for background tasks and cancels them on drop.
#[derive(Default)]
struct BgRefresh(std::sync::Mutex<Vec<AbortHandle>>);

impl BgRefresh {
    /// Registers an abort handle to be cancelled when this guard is dropped.
    fn register(&self, handle: AbortHandle) {
        if let Ok(mut handles) = self.0.lock() {
            handles.push(handle);
        }
    }
}

impl Drop for BgRefresh {
    fn drop(&mut self) {
        if let Ok(mut handles) = self.0.lock() {
            for handle in handles.drain(..) {
                handle.abort();
            }
        }
    }
}
