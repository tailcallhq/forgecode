use std::sync::Arc;

use anyhow::Result;
use forge_app::domain::{
    ChatCompletionMessage, Context as ChatContext, Model, ModelId, Provider, ProviderResponse,
    ResultStream,
};
use forge_app::{EnvironmentInfra, HttpInfra};
use forge_domain::ChatRepository;
use url::Url;

use crate::provider::anthropic::AnthropicResponseRepository;
use crate::provider::openai::OpenAIResponseRepository;

/// Command Code serves multiple protocols from one host: Claude models answer
/// only on the Anthropic `/messages` route, while every other model answers on
/// `/chat/completions`. Routes by model ID and derives the endpoint from the
/// configured chat completions URL.
pub struct CommandCodeResponseRepository<F> {
    openai_repo: OpenAIResponseRepository<F>,
    anthropic_repo: AnthropicResponseRepository<F>,
}

impl<F: HttpInfra + EnvironmentInfra<Config = forge_config::ForgeConfig> + Sync>
    CommandCodeResponseRepository<F>
{
    /// # Arguments
    ///
    /// * `infra` - Infrastructure providing environment and HTTP capabilities
    pub fn new(infra: Arc<F>) -> Self {
        Self {
            openai_repo: OpenAIResponseRepository::new(infra.clone()),
            anthropic_repo: AnthropicResponseRepository::new(infra),
        }
    }

    pub async fn chat(
        &self,
        model_id: &ModelId,
        context: ChatContext,
        provider: Provider<Url>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        let adapted_provider = build_provider(&provider, model_id);
        if is_anthropic_model(model_id) {
            self.anthropic_repo
                .chat(model_id, context, adapted_provider)
                .await
        } else {
            self.openai_repo
                .chat(model_id, context, adapted_provider)
                .await
        }
    }

    /// Fetches the model list from the OpenAI-compatible `/models` endpoint.
    ///
    /// The endpoint reports every model the account can use, including Claude
    /// models that only answer on `/messages`.
    pub async fn models(&self, provider: Provider<Url>) -> Result<Vec<Model>> {
        self.openai_repo.models(provider).await
    }
}

/// Returns `true` when the model answers only on the Anthropic messages route.
fn is_anthropic_model(model_id: &ModelId) -> bool {
    model_id.as_str().starts_with("claude-")
}

/// Rewrites the configured chat completions URL to the Anthropic messages
/// route for Claude models.
fn build_provider(provider: &Provider<Url>, model_id: &ModelId) -> Provider<Url> {
    let mut new_provider = provider.clone();
    if is_anthropic_model(model_id) {
        new_provider.url = messages_url(&provider.url);
        new_provider.response = Some(ProviderResponse::Anthropic);
    }
    new_provider
}

/// Derives the messages endpoint from a chat completions endpoint URL.
fn messages_url(chat_url: &Url) -> Url {
    let base = chat_url
        .as_str()
        .trim_end_matches('/')
        .trim_end_matches("/chat/completions");
    Url::parse(&format!("{base}/messages")).unwrap()
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_model_routing() {
        let fixture = [
            ("claude-opus-5", true),
            ("claude-sonnet-5-5", true),
            ("gpt-6-astra", false),
            ("deepseek/deepseek-v4-pro", false),
            ("z-ai/glm-5.3-flash", false),
        ];

        let actual: Vec<bool> = fixture
            .iter()
            .map(|(model_id, _)| is_anthropic_model(&ModelId::new(*model_id)))
            .collect();
        let expected: Vec<bool> = fixture.iter().map(|(_, expected)| *expected).collect();

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_messages_url() {
        let fixture =
            Url::parse("https://api.commandcode.ai/provider/v1/chat/completions").unwrap();

        let actual = messages_url(&fixture);

        let expected = Url::parse("https://api.commandcode.ai/provider/v1/messages").unwrap();
        assert_eq!(actual, expected);
    }
}
