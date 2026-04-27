use std::sync::Arc;

use derive_setters::Setters;
use forge_domain::{
    ChatCompletionMessageFull, Context, ContextMessage, ConversationId, ModelId, ProviderId,
    ReasoningConfig, ResponseFormat, ResultStreamExt, UserPrompt,
};
use forge_tracker::{AiGenerationPayload, EventKind};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::TemplateEngine;
use crate::agent::AgentService as AS;
use crate::hooks::tracing::TRACKER;

/// Structured response for title generation using JSON format
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(title = "title")]
pub struct TitleResponse {
    /// The generated title for the conversation
    pub title: String,
}

/// Service for generating contextually appropriate titles
#[derive(Setters)]
pub struct TitleGenerator<S> {
    /// Shared reference to the agent services used for AI interactions
    services: Arc<S>,
    /// The user prompt to generate a title for
    user_prompt: UserPrompt,
    /// The model ID to use for title generation
    model_id: ModelId,
    /// Reasoning configuration for the generator.
    reasoning: Option<ReasoningConfig>,
    /// The provider ID to use for title generation
    provider_id: Option<ProviderId>,
}

impl<S: AS> TitleGenerator<S> {
    pub fn new(
        services: Arc<S>,
        user_prompt: UserPrompt,
        model_id: ModelId,
        provider_id: Option<ProviderId>,
    ) -> Self {
        Self {
            services,
            user_prompt,
            model_id,
            reasoning: None,
            provider_id,
        }
    }

    pub async fn generate(&self) -> anyhow::Result<Option<String>> {
        let template = TemplateEngine::default().render(
            "forge-system-prompt-title-generation.md",
            &Default::default(),
        )?;

        let prompt = format!("<user_prompt>{}</user_prompt>", self.user_prompt.as_str());

        // Generate JSON schema from TitleResponse using schemars
        let schema = schemars::schema_for!(TitleResponse);

        let mut ctx = Context::default()
            .temperature(1.0f32)
            .conversation_id(ConversationId::generate())
            .add_message(ContextMessage::system(template))
            .add_message(ContextMessage::user(prompt, Some(self.model_id.clone())))
            .response_format(ResponseFormat::JsonSchema(Box::new(schema)));

        // Set the reasoning if configured.
        if let Some(reasoning) = self.reasoning.as_ref() {
            ctx = ctx.reasoning(reasoning.clone());
        }

        let conversation_id = ctx.conversation_id;
        let stream = self
            .services
            .chat_agent(&self.model_id, ctx, self.provider_id.clone())
            .await?;
        let ChatCompletionMessageFull { content, usage, .. } = stream.into_full(false).await?;

        // Dispatch AI generation event with LLM telemetry
        if let Some(tracker) = TRACKER.get() {
            let provider = self
                .provider_id
                .as_ref()
                .map(|p| p.to_string())
                .unwrap_or_default();
            let payload = AiGenerationPayload {
                provider,
                model: self.model_id.as_str().to_string(),
                input_tokens: *usage.prompt_tokens,
                output_tokens: *usage.completion_tokens,
                latency_ms: 0.0,
                cost: usage.cost,
                conversation_id: conversation_id.map(|c| c.to_string()).unwrap_or_default(),
            };
            let tracker = tracker.clone();
            tokio::spawn(async move {
                let _ = tracker.dispatch(EventKind::AiGeneration(payload)).await;
            });
        }

        // Parse the response - try JSON first (structured output), fallback to plain
        // text
        match serde_json::from_str::<TitleResponse>(&content) {
            Ok(response) => Ok(Some(response.title)),
            Err(_) => {
                // Fallback: Some providers don't support structured output, treat as plain text
                Ok(Some(content.trim().to_string()))
            }
        }
    }
}
