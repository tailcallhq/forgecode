//! LLM-based context summarization service.
//!
//! This module provides semantic summarization of conversation context using
//! an LLM, offering higher quality summaries than template-based extraction.

use std::time::Duration;

use anyhow::Context as _;
use forge_domain::{
    Compact, Context, ContextMessage, ContextSummary, ModelId, Provider, ResultStreamExt,
};
use url::Url;

use crate::{ProviderService, TemplateEngine};
use tracing::{info, warn};

/// LLM-based summarizer for context compaction.
/// LLM-based summarizer for context compaction.
///
/// This service generates semantic summaries of conversation context using
/// an LLM, providing higher quality summaries than template-based extraction.
pub struct LlmSummarizer {
    compact: Compact,
    template_engine: TemplateEngine<'static>,
    timeout: Duration,
    enabled: bool,
}

impl Default for LlmSummarizer {
    fn default() -> Self {
        Self::new(Compact::default())
    }
}

impl LlmSummarizer {
    /// Create a new LLM summarizer with the given configuration
    pub fn new(compact: Compact) -> Self {
        let timeout = Duration::from_secs(compact.summary_timeout_secs);
        Self {
            compact,
            template_engine: TemplateEngine::default(),
            timeout,
            enabled: true,
        }
    }
    /// Enable or disable LLM summarization
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if summarization is enabled (regardless of strategy)
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Check if LLM summarization will be used for the current strategy
    pub fn uses_llm(&self) -> bool {
        self.enabled && self.compact.summarization_strategy.requires_llm()
    }

    /// Generate a summary using the configured strategy.
    ///
    /// Returns the summary text, or an error if summarization fails.
    pub async fn generate_summary<S: ProviderService>(
        &self,
        context_summary: &ContextSummary,
        services: &S,
        provider: Provider<Url>,
    ) -> anyhow::Result<String> {
        match self.compact.summarization_strategy {
            forge_domain::SummarizationStrategy::Extract => {
                self.generate_template_summary(context_summary)
            }
            forge_domain::SummarizationStrategy::Llm => {
                self.generate_llm_summary(context_summary, services, provider)
                    .await
            }
            forge_domain::SummarizationStrategy::Hybrid => {
                // Try LLM first, fall back to template on error
                match self.generate_llm_summary(context_summary, services, provider).await {
                    Ok(summary) => Ok(summary),
                    Err(e) => {
                        warn!("LLM summarization failed, falling back to template: {}", e);
                        self.generate_template_summary(context_summary)
                    }
                }
            }
        }
    }

    /// Generate a summary using template-based extraction.
    fn generate_template_summary(&self, context_summary: &ContextSummary) -> anyhow::Result<String> {
        self.template_engine.render(
            "forge-partial-summary-frame.md",
            &serde_json::json!({"messages": context_summary.messages}),
        )
    }

    /// Generate a summary using LLM.
    async fn generate_llm_summary<S: ProviderService>(
        &self,
        context_summary: &ContextSummary,
        services: &S,
        provider: Provider<Url>,
    ) -> anyhow::Result<String> {
        if !self.enabled {
            return self.generate_template_summary(context_summary);
        }

        let model_id = self
            .compact
            .summary_model
            .clone()
            .unwrap_or_else(|| ModelId::new("claude-sonnet-4-20250514"));

        info!(
            model = %model_id,
            timeout_secs = self.timeout.as_secs(),
            "Generating LLM summary"
        );

        // Build the prompt
        let prompt = self.build_summarization_prompt(context_summary);

        // Create a minimal context with just the prompt
        let prompt_context = Context::default()
            .add_message(ContextMessage::user(prompt, None));

        // Make the LLM call with timeout
        let summary = tokio::time::timeout(
            self.timeout,
            services.chat(&model_id, prompt_context, provider),
        )
        .await
        .with_context(|| "LLM summarization timed out")?
        .with_context(|| "LLM summarization failed")?;

        // Extract the text content from the response
        let summary_message = summary.into_full(false).await?;
        let summary_text = summary_message.content.as_str().to_string();

        info!(
            summary_tokens = context_summary.messages.len(),
            "Generated LLM summary successfully"
        );

        Ok(summary_text)
    }

    /// Build the summarization prompt from the context summary.
    fn build_summarization_prompt(&self, context_summary: &ContextSummary) -> String {
        // Choose template based on available space
        let template_name = if self.compact.summary_max_tokens.unwrap_or(500) <= 200 {
            "forge-summarization-prompt-compact.md"
        } else {
            "forge-summarization-prompt.md"
        };

        match self.template_engine.render(
            template_name,
            &serde_json::json!({"messages": context_summary.messages}),
        ) {
            Ok(prompt) => prompt,
            Err(e) => {
                // Fallback to a simple prompt
                warn!("Failed to render summarization template: {}", e);
                format!(
                    "Summarize the following conversation in 200 tokens or less:\n\n{}",
                    context_summary
                        .messages
                        .iter()
                        .take(10)
                        .map(|m| format!("{:?}: {:?}", m.role, m.contents))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
        }
    }
}

/// Extension trait for Compact to add summarization strategy checks
pub trait SummarizationStrategyExt {
    /// Check if strategy uses LLM
    fn is_llm(&self) -> bool;

    /// Check if strategy uses template extraction
    fn is_extract(&self) -> bool;

    /// Check if strategy is hybrid (try LLM, fallback to extract)
    fn is_hybrid(&self) -> bool;
}

impl SummarizationStrategyExt for forge_domain::SummarizationStrategy {
    fn is_llm(&self) -> bool {
        matches!(self, forge_domain::SummarizationStrategy::Llm)
    }

    fn is_extract(&self) -> bool {
        matches!(self, forge_domain::SummarizationStrategy::Extract)
    }

    fn is_hybrid(&self) -> bool {
        matches!(self, forge_domain::SummarizationStrategy::Hybrid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_summarization_strategy_ext() {
        use forge_domain::SummarizationStrategy;
        assert!(SummarizationStrategy::Extract.is_extract());
        assert!(!SummarizationStrategy::Extract.is_llm());
        assert!(!SummarizationStrategy::Extract.is_hybrid());

        assert!(SummarizationStrategy::Llm.is_llm());
        assert!(!SummarizationStrategy::Llm.is_extract());
        assert!(!SummarizationStrategy::Llm.is_hybrid());

        assert!(SummarizationStrategy::Hybrid.is_hybrid());
        assert!(!SummarizationStrategy::Hybrid.is_extract());
        assert!(!SummarizationStrategy::Hybrid.is_llm());
    }

    #[test]
    fn test_llm_summarizer_default() {
        let summarizer = LlmSummarizer::default();
        assert!(summarizer.is_enabled()); // Default is enabled with Extract strategy
    }

    #[test]
    fn test_llm_summarizer_disabled() {
        use forge_domain::SummarizationStrategy;
        let compact = Compact::new().summarization_strategy(SummarizationStrategy::Llm);
        let mut summarizer = LlmSummarizer::new(compact);
        summarizer.set_enabled(false);
        assert!(!summarizer.is_enabled());
    }
}
