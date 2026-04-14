use std::sync::Arc;

use anyhow::{Context as _, Result};
use aws_sdk_bedrockruntime::Client;
use aws_sdk_bedrockruntime::config::Token;
use forge_config::RetryConfig;
use forge_domain::{
    AuthDetails, ChatCompletionMessage, ChatRepository, Context, Model, ModelId, Provider,
    ResultStream, Transformer,
};
use reqwest::Url;
use tokio::sync::OnceCell;
use tokio_stream::StreamExt;

use crate::provider::bedrock_cache::SetCache;
use crate::provider::bedrock_sanitize_ids::SanitizeToolIds;
use crate::provider::retry::into_retry;
use crate::provider::{FromDomain, IntoDomain};

/// Provider implementation for Amazon Bedrock using Bearer token authentication
///
/// This provider uses the AWS SDK with Bearer token authentication instead of
/// AWS SigV4 signing, allowing it to work with Bedrock Access Gateway.
struct BedrockProvider {
    provider: Provider<Url>,
    region: String,
    client: OnceCell<Client>,
}

impl BedrockProvider {
    /// Creates a new BedrockProvider instance
    ///
    /// Credentials are loaded from the provider's credential:
    /// - API key field: Bearer token for Bedrock Access Gateway
    /// - URL params: AWS_REGION (defaults to us-east-1)
    pub fn new(provider: Provider<Url>) -> Result<Self> {
        // Validate credentials are present
        let credential = provider
            .credential
            .as_ref()
            .context("Bedrock requires credentials")?;

        // Validate API key (bearer token)
        match &credential.auth_details {
            AuthDetails::ApiKey(key) if !key.is_empty() => {}
            _ => anyhow::bail!("Bearer token is required in API key field"),
        }

        // Extract region from URL params
        let region_param: forge_domain::URLParam = "AWS_REGION".to_string().into();
        let region = credential
            .url_params
            .get(&region_param)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "us-east-1".to_string());

        Ok(Self { provider, region, client: OnceCell::new() })
    }

    /// Initializes and returns the AWS Bedrock client
    ///
    /// The client is lazily initialized on first call and reused for subsequent
    /// calls. This avoids creating the client during tests that only validate
    /// configuration. Uses async locking to ensure thread-safe initialization.
    ///
    /// # Errors
    ///
    /// Returns an error if the bearer token cannot be retrieved from
    /// credentials
    async fn init(&self) -> Result<&Client> {
        self.client
            .get_or_try_init(|| async {
                // Get the bearer token from provider credentials
                let bearer_token = self
                    .provider
                    .credential
                    .as_ref()
                    .and_then(|c| match &c.auth_details {
                        AuthDetails::ApiKey(key) if !key.is_empty() => {
                            Some(key.as_ref().to_string())
                        }
                        _ => None,
                    })
                    .context("Bearer token is required in API key field")?;

                // Configure AWS SDK client with Bearer token authentication
                let config = aws_sdk_bedrockruntime::Config::builder()
                    .region(aws_sdk_bedrockruntime::config::Region::new(
                        self.region.clone(),
                    ))
                    .bearer_token(Token::new(bearer_token, None))
                    .build();

                Ok(aws_sdk_bedrockruntime::Client::from_conf(config))
            })
            .await
    }

    /// Check if the model supports prompt caching
    ///
    /// AWS Bedrock supports prompt caching for models that implement cache
    /// points. Currently supported models:
    /// - Anthropic Claude (all variants) - System + Message cache points
    /// - Amazon Nova (all variants) - System cache points only (20K token
    ///   limit)
    ///
    /// The SetCache transformer is model-aware and will only add message-level
    /// cache points for Claude models.
    fn supports_caching(model_id: &str) -> bool {
        let model_lower = model_id.to_lowercase();

        // Claude and Nova models support prompt caching
        // SetCache is model-aware: adds message cache points only for Claude
        model_lower.contains("anthropic") || model_lower.contains("claude")
    }

    /// Transform model ID with regional prefix if needed
    pub fn transform_model_id(&self, model_id: &str) -> String {
        // Skip if already has global prefix
        if model_id.starts_with("global.") {
            return model_id.to_string();
        }

        // Determine regional prefix
        let prefix = match self.region.as_str() {
            r if r.starts_with("us-") && !r.contains("gov") => "us.",
            r if r.starts_with("eu-") => "eu.",
            "ap-southeast-2" => "au.",
            r if r.starts_with("ap-") => "apac.",
            _ => "",
        };

        // Only prefix Anthropic models that don't already have a regional prefix
        if model_id.contains("anthropic.")
            && !model_id.starts_with("us.")
            && !model_id.starts_with("eu.")
            && !model_id.starts_with("apac.")
            && !model_id.starts_with("au.")
        {
            format!("{}{}", prefix, model_id)
        } else {
            model_id.to_string()
        }
    }

    /// Checks if a ConverseStreamError service error is retryable
    fn is_retryable_converse_error(
        err: &aws_sdk_bedrockruntime::operation::converse_stream::ConverseStreamError,
    ) -> bool {
        use aws_sdk_bedrockruntime::operation::converse_stream::ConverseStreamError;
        matches!(
            err,
            ConverseStreamError::ThrottlingException(_)
                | ConverseStreamError::ServiceUnavailableException(_)
                | ConverseStreamError::InternalServerException(_)
                | ConverseStreamError::ModelStreamErrorException(_)
                | ConverseStreamError::ModelNotReadyException(_)
        )
    }

    /// Checks if a ConverseStreamOutputError service error is retryable
    fn is_retryable_stream_output_error(
        err: &aws_sdk_bedrockruntime::types::error::ConverseStreamOutputError,
    ) -> bool {
        use aws_sdk_bedrockruntime::types::error::ConverseStreamOutputError;
        matches!(
            err,
            ConverseStreamOutputError::ThrottlingException(_)
                | ConverseStreamOutputError::ServiceUnavailableException(_)
                | ConverseStreamOutputError::InternalServerException(_)
                | ConverseStreamOutputError::ModelStreamErrorException(_)
        )
    }

    /// Checks if an SDK error is retryable based on error type (network/timeout
    /// errors)
    fn is_retryable_sdk_error<E, R>(err: &aws_sdk_bedrockruntime::error::SdkError<E, R>) -> bool {
        use aws_sdk_bedrockruntime::error::SdkError;
        matches!(
            err,
            SdkError::TimeoutError(_) | SdkError::DispatchFailure(_)
        )
    }

    /// Perform a streaming chat completion
    pub async fn chat(
        &self,
        model: &ModelId,
        context: Context,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        let model_id = self.transform_model_id(model.as_str());

        // Convert context to AWS SDK types using FromDomain trait
        let bedrock_input =
            aws_sdk_bedrockruntime::operation::converse_stream::ConverseStreamInput::from_domain(
                context,
            )
            .context("Failed to convert context to Bedrock ConverseStreamInput")?;

        // Apply transformers pipeline
        let supports_caching = Self::supports_caching(&model_id);
        let bedrock_input = SetCache
            .when(move |_| supports_caching)
            .pipe(SanitizeToolIds)
            .transform(bedrock_input);

        // Build and send the converse_stream request
        let output = self
            .init()
            .await?
            .converse_stream()
            .model_id(model_id)
            .set_system(bedrock_input.system.clone())
            .set_messages(bedrock_input.messages.clone())
            .set_tool_config(bedrock_input.tool_config.clone())
            .set_inference_config(bedrock_input.inference_config.clone())
            .set_additional_model_request_fields(
                bedrock_input.additional_model_request_fields.clone(),
            )
            .send()
            .await
            .map_err(|sdk_error| {
                use aws_sdk_bedrockruntime::error::SdkError;

                // Check if this is a retryable error by matching on SDK error types
                let is_retryable = match &sdk_error {
                    SdkError::ServiceError(err) => Self::is_retryable_converse_error(err.err()),
                    _ => Self::is_retryable_sdk_error(&sdk_error),
                };

                // Extract the source error for better error messages
                // SAFETY: into_source() always returns Ok for all SdkError variants
                // (see aws-smithy-runtime-api/src/client/result.rs:448-459)
                let source = sdk_error.into_source().unwrap();

                if is_retryable {
                    forge_domain::Error::Retryable(anyhow::anyhow!("{}", source)).into()
                } else {
                    anyhow::anyhow!("{}", source)
                }
            })?;

        // Convert the Bedrock event stream to ChatCompletionMessage stream
        let stream = futures::stream::unfold(output.stream, |mut event_stream| async move {
            match event_stream.recv().await {
                Ok(Some(event)) => {
                    let message = event.into_domain();
                    Some((Ok(message), event_stream))
                }
                Ok(None) => None, // End of stream
                Err(stream_error) => {
                    use aws_sdk_bedrockruntime::error::SdkError;

                    // Check if this is a retryable stream error by matching on SDK error types
                    let is_retryable = match &stream_error {
                        SdkError::ServiceError(err) => {
                            Self::is_retryable_stream_output_error(err.err())
                        }
                        _ => Self::is_retryable_sdk_error(&stream_error),
                    };

                    let error = if is_retryable {
                        forge_domain::Error::Retryable(anyhow::anyhow!(
                            "Bedrock stream error: {:?}",
                            stream_error
                        ))
                        .into()
                    } else {
                        anyhow::anyhow!("Bedrock stream error: {:?}", stream_error)
                    };
                    Some((Err(error), event_stream))
                }
            }
        });

        Ok(Box::pin(stream))
    }

    /// Get available models
    pub async fn models(&self) -> Result<Vec<Model>> {
        // Bedrock doesn't have a models list API
        // Return hardcoded models from configuration
        match &self.provider.models {
            Some(forge_domain::ModelSource::Hardcoded(models)) => Ok(models.clone()),
            _ => Ok(vec![]),
        }
    }
}

/// Converts Bedrock stream events to ChatCompletionMessage
impl IntoDomain for aws_sdk_bedrockruntime::types::ConverseStreamOutput {
    type Domain = forge_domain::ChatCompletionMessage;

    fn into_domain(self) -> Self::Domain {
        use aws_sdk_bedrockruntime::types::ConverseStreamOutput;
        use forge_domain::{
            ChatCompletionMessage, Content, FinishReason, ToolCallId, ToolCallPart, ToolName,
        };

        match self {
            ConverseStreamOutput::ContentBlockDelta(delta) => {
                if let Some(delta_content) = delta.delta {
                    match delta_content {
                        aws_sdk_bedrockruntime::types::ContentBlockDelta::Text(text) => {
                            ChatCompletionMessage::assistant(Content::part(text))
                        }
                        aws_sdk_bedrockruntime::types::ContentBlockDelta::ToolUse(tool_use) => {
                            // Tool use delta - partial JSON for tool arguments
                            ChatCompletionMessage::assistant(Content::part("")).add_tool_call(
                                ToolCallPart {
                                    call_id: None,
                                    name: None,
                                    arguments_part: tool_use.input,
                                    thought_signature: None,
                                },
                            )
                        }
                        aws_sdk_bedrockruntime::types::ContentBlockDelta::ReasoningContent(
                            reasoning,
                        ) => {
                            // Handle reasoning content delta
                            match reasoning {
                                aws_sdk_bedrockruntime::types::ReasoningContentBlockDelta::Text(
                                    text,
                                ) => {
                                    // Reasoning text - add to both reasoning field and as detail part
                                    ChatCompletionMessage::default()
                                        .reasoning(Content::part(text.clone()))
                                        .add_reasoning_detail(forge_domain::Reasoning::Part(vec![
                                            forge_domain::ReasoningPart {
                                                text: Some(text),
                                                signature: None,
                                                ..Default::default()
                                            },
                                        ]))
                                }
                                aws_sdk_bedrockruntime::types::ReasoningContentBlockDelta::Signature(
                                    sig,
                                ) => {
                                    // Signature for reasoning - add as reasoning detail part
                                    ChatCompletionMessage::default().add_reasoning_detail(
                                        forge_domain::Reasoning::Part(vec![
                                            forge_domain::ReasoningPart {
                                                text: None,
                                                signature: Some(sig),
                                                ..Default::default()
                                            },
                                        ]),
                                    )
                                }
                                aws_sdk_bedrockruntime::types::ReasoningContentBlockDelta::RedactedContent(_) => {
                                    // Redacted content - skip it
                                    ChatCompletionMessage::default()
                                }
                                _ => ChatCompletionMessage::default(),
                            }
                        }
                        _ => ChatCompletionMessage::assistant(Content::part("")),
                    }
                } else {
                    ChatCompletionMessage::assistant(Content::part(""))
                }
            }
            ConverseStreamOutput::ContentBlockStart(start) => {
                if let Some(start_content) = start.start {
                    match start_content {
                        aws_sdk_bedrockruntime::types::ContentBlockStart::ToolUse(tool_use) => {
                            // Tool use start - contains tool name and ID
                            ChatCompletionMessage::assistant(Content::part("")).add_tool_call(
                                ToolCallPart {
                                    call_id: Some(ToolCallId::new(tool_use.tool_use_id)),
                                    name: Some(ToolName::new(tool_use.name)),
                                    arguments_part: String::new(),
                                    thought_signature: None,
                                },
                            )
                        }
                        _ => ChatCompletionMessage::assistant(Content::part("")),
                    }
                } else {
                    ChatCompletionMessage::assistant(Content::part(""))
                }
            }
            ConverseStreamOutput::MessageStop(stop) => {
                // Message stop contains finish reason
                let finish_reason = match &stop.stop_reason {
                    aws_sdk_bedrockruntime::types::StopReason::EndTurn => FinishReason::Stop,
                    aws_sdk_bedrockruntime::types::StopReason::MaxTokens => FinishReason::Length,
                    aws_sdk_bedrockruntime::types::StopReason::ToolUse => FinishReason::ToolCalls,
                    aws_sdk_bedrockruntime::types::StopReason::ContentFiltered => {
                        FinishReason::ContentFilter
                    }
                    _ => FinishReason::Stop,
                };

                ChatCompletionMessage::assistant(Content::part(""))
                    .finish_reason_opt(Some(finish_reason))
            }
            ConverseStreamOutput::Metadata(metadata) => {
                // Metadata contains usage information
                let usage = metadata.usage.map(|u| {
                    // AWS Bedrock supports cache tokens but not reasoning tokens
                    // Sum both cache read and cache write tokens into cached_tokens field
                    let cached_tokens = u
                        .cache_read_input_tokens
                        .unwrap_or(0)
                        .saturating_add(u.cache_write_input_tokens.unwrap_or(0));

                    forge_domain::Usage {
                        prompt_tokens: forge_domain::TokenCount::Actual(u.input_tokens as usize),
                        completion_tokens: forge_domain::TokenCount::Actual(
                            u.output_tokens as usize,
                        ),
                        total_tokens: forge_domain::TokenCount::Actual(u.total_tokens as usize),
                        cached_tokens: forge_domain::TokenCount::Actual(cached_tokens as usize),
                        ..Default::default()
                    }
                });

                let mut msg = ChatCompletionMessage::assistant(Content::part(""));
                if let Some(u) = usage {
                    msg = msg.usage(u);
                }
                msg
            }
            ConverseStreamOutput::ContentBlockStop(_) => {
                ChatCompletionMessage::assistant("").finish_reason(FinishReason::Stop)
            }
            _ => ChatCompletionMessage::assistant(Content::part("")),
        }
    }
}

/// Converts domain Context to Bedrock ConverseStreamInput
impl FromDomain<forge_domain::Context>
    for aws_sdk_bedrockruntime::operation::converse_stream::ConverseStreamInput
{
    fn from_domain(context: forge_domain::Context) -> anyhow::Result<Self> {
        use aws_sdk_bedrockruntime::operation::converse_stream::ConverseStreamInput;
        use aws_sdk_bedrockruntime::types::{InferenceConfiguration, Message, SystemContentBlock};

        // Convert system messages
        let system: Vec<SystemContentBlock> = context
            .messages
            .iter()
            .filter_map(|msg| match &msg.message {
                forge_domain::ContextMessage::Text(text_msg)
                    if text_msg.has_role(forge_domain::Role::System) =>
                {
                    Some(SystemContentBlock::Text(text_msg.content.clone()))
                }
                _ => None,
            })
            .collect();

        // Convert user and assistant messages
        // Group consecutive tool results into single User messages as required by
        // Bedrock API
        let messages: Vec<Message> = {
            let mut result = Vec::new();
            let mut pending_tool_results: Vec<forge_domain::ContextMessage> = Vec::new();

            for message in context.messages.into_iter() {
                if message.has_role(forge_domain::Role::System) {
                    continue;
                }

                match &message.message {
                    forge_domain::ContextMessage::Tool(_) => {
                        // Accumulate tool results
                        pending_tool_results.push(message.message);
                    }
                    _ => {
                        // Flush pending tool results before processing non-tool message
                        if !pending_tool_results.is_empty() {
                            let tool_results: Vec<_> = std::mem::take(&mut pending_tool_results);
                            result.push(Message::from_domain(tool_results)?);
                        }

                        // Convert and add the non-tool message
                        result.push(
                            Message::from_domain(message.message)
                                .with_context(|| "Failed to convert message to Bedrock format")?,
                        );
                    }
                }
            }

            // Flush any remaining tool results
            if !pending_tool_results.is_empty() {
                result.push(Message::from_domain(pending_tool_results)?);
            }

            Ok::<Vec<Message>, anyhow::Error>(result)
        }?;

        // Convert tool configuration
        let tool_config = if !context.tools.is_empty() {
            use aws_sdk_bedrockruntime::types::{Tool, ToolChoice, ToolConfiguration};

            let tool_specs: Vec<Tool> = context
                .tools
                .into_iter()
                .map(Tool::from_domain)
                .collect::<anyhow::Result<Vec<_>>>()?;

            let choice = context
                .tool_choice
                .filter(|c| !matches!(c, forge_domain::ToolChoice::None))
                .map(ToolChoice::from_domain)
                .transpose()?;

            Some(
                ToolConfiguration::builder()
                    .set_tools(Some(tool_specs))
                    .set_tool_choice(choice)
                    .build()
                    .map_err(|e| anyhow::anyhow!("Failed to build tool configuration: {}", e))?,
            )
        } else {
            None
        };

        // Convert inference configuration
        // When extended thinking is enabled, top_p must be >= 0.95 or unset
        let has_thinking = context
            .reasoning
            .as_ref()
            .and_then(|r| r.enabled)
            .unwrap_or(false);
        let adjusted_top_p = if has_thinking {
            // If thinking is enabled and top_p is set, ensure it's at least 0.95
            context.top_p.map(|p| {
                let value = p.value();
                if value < 0.95 {
                    forge_domain::TopP::new(0.95).unwrap()
                } else {
                    p
                }
            })
        } else {
            context.top_p
        };

        let inference_config = if context.temperature.is_some()
            || adjusted_top_p.is_some()
            || context.top_k.is_some()
            || context.max_tokens.is_some()
        {
            Some(
                InferenceConfiguration::builder()
                    .set_temperature(context.temperature.map(|t| t.value()))
                    .set_top_p(adjusted_top_p.map(|t| t.value()))
                    .set_max_tokens(context.max_tokens.map(|t| t as i32))
                    .build(),
            )
        } else {
            None
        };

        // Convert reasoning configuration to additional model request fields
        // For Claude models with extended thinking support
        // Based on AWS Bedrock docs: additionalModelRequestFields for Claude extended
        // thinking https://docs.aws.amazon.com/bedrock/latest/userguide/model-parameters-anthropic-claude-messages.html
        let additional_model_fields = if let Some(reasoning_config) = &context.reasoning {
            if reasoning_config.enabled.unwrap_or(false) {
                let mut thinking_config = std::collections::HashMap::new();
                thinking_config.insert(
                    "type".to_string(),
                    aws_smithy_types::Document::String("enabled".to_string()),
                );

                // Set budget_tokens (REQUIRED when thinking is enabled)
                // The budget_tokens parameter determines the maximum number of tokens
                // Claude is allowed to use for its internal reasoning process
                // Default to 4000 if not specified (AWS recommendation for good quality)
                let budget_tokens = reasoning_config.max_tokens.unwrap_or(4000);
                thinking_config.insert(
                    "budget_tokens".to_string(),
                    aws_smithy_types::Document::Number(aws_smithy_types::Number::PosInt(
                        budget_tokens as u64,
                    )),
                );

                let mut fields = std::collections::HashMap::new();
                fields.insert(
                    "thinking".to_string(),
                    aws_smithy_types::Document::Object(thinking_config),
                );

                Some(aws_smithy_types::Document::Object(fields))
            } else {
                None
            }
        } else {
            None
        };

        let builder = ConverseStreamInput::builder()
            .set_system(if system.is_empty() {
                None
            } else {
                Some(system)
            })
            .set_messages(Some(messages))
            .set_tool_config(tool_config)
            .set_inference_config(inference_config)
            .set_additional_model_request_fields(additional_model_fields);

        builder
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build Bedrock ConverseStreamInput: {}", e))
    }
}

/// Converts multiple tool results into a single Bedrock User message
///
/// Bedrock requires all tool results for a given assistant message's tool calls
/// to be in a single User message with multiple ToolResult content blocks.
impl FromDomain<Vec<forge_domain::ContextMessage>> for aws_sdk_bedrockruntime::types::Message {
    fn from_domain(tool_results: Vec<forge_domain::ContextMessage>) -> anyhow::Result<Self> {
        use aws_sdk_bedrockruntime::types::{
            ContentBlock, ConversationRole, Message, ToolResultBlock, ToolResultContentBlock,
            ToolResultStatus,
        };

        if tool_results.is_empty() {
            anyhow::bail!("Cannot create message from empty tool results");
        }

        let mut content_blocks = Vec::new();

        for msg in tool_results {
            match msg {
                forge_domain::ContextMessage::Tool(tool_result) => {
                    let is_error = tool_result.is_error();
                    let tool_result_block = ToolResultBlock::builder()
                        .tool_use_id(
                            tool_result
                                .call_id
                                .ok_or_else(|| anyhow::anyhow!("Tool result missing call ID"))?
                                .as_str(),
                        )
                        .set_content(Some(vec![ToolResultContentBlock::Text(
                            tool_result
                                .output
                                .as_str()
                                .ok_or_else(|| anyhow::anyhow!("Tool result has no text output"))?
                                .to_string(),
                        )]))
                        .status(if is_error {
                            ToolResultStatus::Error
                        } else {
                            ToolResultStatus::Success
                        })
                        .build()
                        .map_err(|e| anyhow::anyhow!("Failed to build tool result block: {}", e))?;

                    content_blocks.push(ContentBlock::ToolResult(tool_result_block));
                }
                _ => anyhow::bail!("Expected Tool message, got different message type"),
            }
        }

        Message::builder()
            .role(ConversationRole::User)
            .set_content(Some(content_blocks))
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build tool results message: {}", e))
    }
}

/// Converts a domain ContextMessage to a Bedrock Message
impl FromDomain<forge_domain::ContextMessage> for aws_sdk_bedrockruntime::types::Message {
    fn from_domain(msg: forge_domain::ContextMessage) -> anyhow::Result<Self> {
        use anyhow::Context as _;
        use aws_sdk_bedrockruntime::primitives::Blob;
        use aws_sdk_bedrockruntime::types::{
            ContentBlock, ConversationRole, ImageBlock, ImageSource, Message, ToolResultBlock,
            ToolResultContentBlock, ToolResultStatus, ToolUseBlock,
        };

        match msg {
            forge_domain::ContextMessage::Text(text_msg) => {
                let mut content_blocks = Vec::new();

                // Add thought signature FIRST if present (for Assistant messages)
                // AWS requires that when thinking is enabled, assistant messages MUST start
                // with reasoning blocks
                if text_msg.role == forge_domain::Role::Assistant
                    && let Some(reasoning_details) = &text_msg.reasoning_details
                {
                    for reasoning in reasoning_details {
                        use aws_sdk_bedrockruntime::types::{
                            ReasoningContentBlock, ReasoningTextBlock,
                        };

                        let signature = reasoning
                            .signature
                            .clone()
                            .or_else(|| text_msg.thought_signature.clone());

                        if let (Some(text), Some(signature)) = (&reasoning.text, signature) {
                            let reasoning_text_block = ReasoningTextBlock::builder()
                                .text(text.clone())
                                .signature(signature)
                                .build()
                                .map_err(|e| {
                                    anyhow::anyhow!("Failed to build reasoning text block: {}", e)
                                })?;

                            content_blocks.push(ContentBlock::ReasoningContent(
                                ReasoningContentBlock::ReasoningText(reasoning_text_block),
                            ));
                        } else if let Some(data) = &reasoning.data {
                            content_blocks.push(ContentBlock::ReasoningContent(
                                ReasoningContentBlock::RedactedContent(Blob::new(
                                    data.clone().into_bytes(),
                                )),
                            ));
                        }
                    }
                }

                // Add text content if not empty
                if !text_msg.content.is_empty() {
                    content_blocks.push(ContentBlock::Text(text_msg.content.clone()));
                }

                // Add tool calls if present
                if let Some(tool_calls) = text_msg.tool_calls {
                    for tool_call in tool_calls {
                        let tool_use = ToolUseBlock::builder()
                            .tool_use_id(
                                tool_call
                                    .call_id
                                    .ok_or_else(|| anyhow::anyhow!("Tool call missing ID"))?
                                    .as_str(),
                            )
                            .name(tool_call.name.to_string())
                            .input(aws_smithy_types::Document::from_domain(
                                tool_call.arguments,
                            )?)
                            .build()
                            .map_err(|e| {
                                anyhow::anyhow!("Failed to build tool use block: {}", e)
                            })?;

                        content_blocks.push(ContentBlock::ToolUse(tool_use));
                    }
                }

                // Map role
                let role = match text_msg.role {
                    forge_domain::Role::User => ConversationRole::User,
                    forge_domain::Role::Assistant => ConversationRole::Assistant,
                    forge_domain::Role::System => {
                        anyhow::bail!("System messages should be filtered out before conversion")
                    }
                };

                Message::builder()
                    .role(role)
                    .set_content(Some(content_blocks))
                    .build()
                    .map_err(|e| anyhow::anyhow!("Failed to build message: {}", e))
            }
            forge_domain::ContextMessage::Tool(tool_result) => {
                let is_error = tool_result.is_error();
                let tool_result_block = ToolResultBlock::builder()
                    .tool_use_id(
                        tool_result
                            .call_id
                            .ok_or_else(|| anyhow::anyhow!("Tool result missing call ID"))?
                            .as_str(),
                    )
                    .set_content(Some(vec![ToolResultContentBlock::Text(
                        tool_result
                            .output
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("Tool result has no text output"))?
                            .to_string(),
                    )]))
                    .status(if is_error {
                        ToolResultStatus::Error
                    } else {
                        ToolResultStatus::Success
                    })
                    .build()
                    .map_err(|e| anyhow::anyhow!("Failed to build tool result block: {}", e))?;

                Message::builder()
                    .role(ConversationRole::User)
                    .content(ContentBlock::ToolResult(tool_result_block))
                    .build()
                    .map_err(|e| anyhow::anyhow!("Failed to build tool result message: {}", e))
            }
            forge_domain::ContextMessage::Image(img) => {
                let image_block = ImageBlock::builder()
                    .source(ImageSource::Bytes(Blob::new(
                        base64::Engine::decode(
                            &base64::engine::general_purpose::STANDARD,
                            img.data(),
                        )
                        .with_context(|| "Failed to decode base64 image data")?,
                    )))
                    .build()
                    .map_err(|e| anyhow::anyhow!("Failed to build image block: {}", e))?;

                Message::builder()
                    .role(ConversationRole::User)
                    .content(ContentBlock::Image(image_block))
                    .build()
                    .map_err(|e| anyhow::anyhow!("Failed to build image message: {}", e))
            }
        }
    }
}

/// Converts schemars Schema to AWS Bedrock ToolInputSchema
impl FromDomain<schemars::Schema> for aws_sdk_bedrockruntime::types::ToolInputSchema {
    fn from_domain(schema: schemars::Schema) -> anyhow::Result<Self> {
        use anyhow::Context as _;
        use aws_sdk_bedrockruntime::types::ToolInputSchema;

        // Serialize Schema to JSON value first
        let json_value =
            serde_json::to_value(&schema).with_context(|| "Failed to serialize Schema")?;

        // Convert JSON value to Document and wrap in ToolInputSchema
        Ok(ToolInputSchema::Json(json_value_to_document(json_value)))
    }
}

/// Converts ToolCallArguments to AWS Smithy Document
impl FromDomain<forge_domain::ToolCallArguments> for aws_smithy_types::Document {
    fn from_domain(args: forge_domain::ToolCallArguments) -> anyhow::Result<Self> {
        use anyhow::Context as _;

        // Parse the arguments to get a serde_json::Value
        let json_value = args
            .parse()
            .with_context(|| "Failed to parse tool call arguments")?;

        // Convert JSON value to Document
        Ok(json_value_to_document(json_value))
    }
}

/// Helper function to convert serde_json::Value to aws_smithy_types::Document
fn json_value_to_document(value: serde_json::Value) -> aws_smithy_types::Document {
    use std::collections::HashMap;

    use aws_smithy_types::{Document, Number};

    match value {
        serde_json::Value::Null => Document::Null,
        serde_json::Value::Bool(b) => Document::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Document::Number(Number::PosInt(i as u64))
            } else if let Some(f) = n.as_f64() {
                Document::Number(Number::Float(f))
            } else {
                Document::Null
            }
        }
        serde_json::Value::String(s) => Document::String(s),
        serde_json::Value::Array(arr) => {
            Document::Array(arr.into_iter().map(json_value_to_document).collect())
        }
        serde_json::Value::Object(obj) => {
            let map: HashMap<String, Document> = obj
                .into_iter()
                .map(|(k, v)| (k, json_value_to_document(v)))
                .collect();
            Document::Object(map)
        }
    }
}

/// Converts domain ToolDefinition to Bedrock Tool
impl FromDomain<forge_domain::ToolDefinition> for aws_sdk_bedrockruntime::types::Tool {
    fn from_domain(tool: forge_domain::ToolDefinition) -> anyhow::Result<Self> {
        use aws_sdk_bedrockruntime::types::{Tool, ToolInputSchema, ToolSpecification};

        let spec = ToolSpecification::builder()
            .name(tool.name.to_string())
            .description(tool.description.clone())
            .input_schema(ToolInputSchema::from_domain(tool.input_schema)?)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build tool specification: {}", e))?;

        Ok(Tool::ToolSpec(spec))
    }
}

/// Converts domain ToolChoice to Bedrock ToolChoice
impl FromDomain<forge_domain::ToolChoice> for aws_sdk_bedrockruntime::types::ToolChoice {
    fn from_domain(choice: forge_domain::ToolChoice) -> anyhow::Result<Self> {
        use aws_sdk_bedrockruntime::types::{
            AnyToolChoice, AutoToolChoice, SpecificToolChoice, ToolChoice,
        };

        let bedrock_choice = match choice {
            forge_domain::ToolChoice::Auto => ToolChoice::Auto(AutoToolChoice::builder().build()),
            forge_domain::ToolChoice::Required => ToolChoice::Any(AnyToolChoice::builder().build()),
            forge_domain::ToolChoice::Call(tool_name) => ToolChoice::Tool(
                SpecificToolChoice::builder()
                    .name(tool_name.to_string())
                    .build()
                    .map_err(|e| anyhow::anyhow!("Failed to build tool choice: {}", e))?,
            ),
            forge_domain::ToolChoice::None => {
                // For None, we'll return a default Auto choice, but the caller should handle
                // this by not setting tool_choice at all
                ToolChoice::Auto(AutoToolChoice::builder().build())
            }
        };

        Ok(bedrock_choice)
    }
}

/// Repository for AWS Bedrock provider responses
pub struct BedrockResponseRepository {
    retry_config: Arc<RetryConfig>,
}

impl BedrockResponseRepository {
    pub fn new(retry_config: Arc<RetryConfig>) -> Self {
        Self { retry_config }
    }
}

#[async_trait::async_trait]
impl ChatRepository for BedrockResponseRepository {
    async fn chat(
        &self,
        model_id: &ModelId,
        context: Context,
        provider: Provider<Url>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        let retry_config = self.retry_config.clone();
        let provider_client =
            BedrockProvider::new(provider).map_err(|e| into_retry(e, &retry_config))?;

        let stream = provider_client
            .chat(model_id, context)
            .await
            .map_err(|e| into_retry(e, &retry_config))?;

        Ok(Box::pin(stream.map(move |item| {
            item.map_err(|e| into_retry(e, &retry_config))
        })))
    }

    async fn models(&self, provider: Provider<Url>) -> anyhow::Result<Vec<Model>> {
        let retry_config = self.retry_config.clone();
        let provider_client = BedrockProvider::new(provider)?;
        provider_client
            .models()
            .await
            .map_err(|e| into_retry(e, &retry_config))
            .context("Failed to fetch models from Bedrock provider")
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::InputModality;
    use pretty_assertions::assert_eq;

    use super::*;

    fn provider_fixture(token: &str, region: Option<&str>) -> Provider<Url> {
        use forge_domain::{
            ApiKey, AuthCredential, AuthDetails, ProviderId, ProviderResponse, ProviderType,
            URLParam, URLParamValue,
        };

        let mut url_params = std::collections::HashMap::new();
        if let Some(r) = region {
            url_params.insert(
                URLParam::from("AWS_REGION".to_string()),
                URLParamValue::from(r.to_string()),
            );
        }

        Provider {
            id: ProviderId::from("bedrock".to_string()),
            provider_type: ProviderType::Llm,
            response: Some(ProviderResponse::Bedrock),
            url: Url::parse("https://bedrock-runtime.us-east-1.amazonaws.com").unwrap(),
            models: None,
            auth_methods: vec![],
            url_params: vec![],
            credential: Some(AuthCredential {
                id: ProviderId::from("bedrock".to_string()),
                auth_details: AuthDetails::ApiKey(ApiKey::from(token.to_string())),
                url_params,
            }),
            custom_headers: None,
        }
    }

    fn bedrock_provider_fixture(region: &str) -> BedrockProvider {
        BedrockProvider {
            provider: provider_fixture("test-token", Some(region)),
            client: OnceCell::new(),
            region: region.to_string(),
        }
    }

    #[test]
    fn test_new_with_valid_credentials() {
        let fixture = provider_fixture("my-bearer-token", Some("eu-central-1"));
        let actual = BedrockProvider::new(fixture);
        assert!(actual.is_ok());
    }

    #[test]
    fn test_new_without_credentials() {
        let mut fixture = provider_fixture("token", None);
        fixture.credential = None;
        let actual = BedrockProvider::new(fixture);
        assert!(actual.is_err());
        assert_eq!(
            actual.err().unwrap().to_string(),
            "Bedrock requires credentials"
        );
    }

    #[test]
    fn test_new_with_empty_token() {
        let fixture = provider_fixture("", None);
        let actual = BedrockProvider::new(fixture);
        assert!(actual.is_err());
        assert_eq!(
            actual.err().unwrap().to_string(),
            "Bearer token is required in API key field"
        );
    }

    #[test]
    fn test_new_defaults_to_us_east_1() {
        let fixture = provider_fixture("token", None);
        let actual = BedrockProvider::new(fixture).unwrap();
        let expected = "us-east-1";
        assert_eq!(actual.region, expected);
    }

    #[test]
    fn test_new_uses_custom_region() {
        let fixture = provider_fixture("token", Some("ap-southeast-2"));
        let actual = BedrockProvider::new(fixture).unwrap();
        let expected = "ap-southeast-2";
        assert_eq!(actual.region, expected);
    }

    #[test]
    fn test_supports_caching_claude() {
        let actual = BedrockProvider::supports_caching("anthropic.claude-3-sonnet");
        assert!(actual);
    }

    #[test]
    fn test_supports_caching_anthropic() {
        let actual = BedrockProvider::supports_caching("anthropic.claude-v2");
        assert!(actual);
    }

    #[test]
    fn test_supports_caching_non_claude() {
        let actual = BedrockProvider::supports_caching("amazon.nova-pro-v1:0");
        assert!(!actual);
    }

    #[test]
    fn test_transform_model_id_us_east() {
        let fixture = bedrock_provider_fixture("us-east-1");
        let actual = fixture.transform_model_id("anthropic.claude-3-5-sonnet-20241022-v2:0");
        let expected = "us.anthropic.claude-3-5-sonnet-20241022-v2:0";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_transform_model_id_us_west() {
        let fixture = bedrock_provider_fixture("us-west-2");
        let actual = fixture.transform_model_id("anthropic.claude-3-opus");
        let expected = "us.anthropic.claude-3-opus";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_transform_model_id_eu_region() {
        let fixture = bedrock_provider_fixture("eu-west-1");
        let actual = fixture.transform_model_id("anthropic.claude-3-5-sonnet-20241022-v2:0");
        let expected = "eu.anthropic.claude-3-5-sonnet-20241022-v2:0";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_transform_model_id_ap_southeast_2() {
        let fixture = bedrock_provider_fixture("ap-southeast-2");
        let actual = fixture.transform_model_id("anthropic.claude-3-haiku");
        let expected = "au.anthropic.claude-3-haiku";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_transform_model_id_ap_northeast() {
        let fixture = bedrock_provider_fixture("ap-northeast-1");
        let actual = fixture.transform_model_id("anthropic.claude-3-sonnet");
        let expected = "apac.anthropic.claude-3-sonnet";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_transform_model_id_already_prefixed() {
        let fixture = bedrock_provider_fixture("us-east-1");
        let actual = fixture.transform_model_id("us.anthropic.claude-3-5-sonnet-20241022-v2:0");
        let expected = "us.anthropic.claude-3-5-sonnet-20241022-v2:0";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_transform_model_id_global_prefix() {
        let fixture = bedrock_provider_fixture("us-east-1");
        let actual = fixture.transform_model_id("global.anthropic.claude-3-opus");
        let expected = "global.anthropic.claude-3-opus";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_transform_model_id_non_anthropic() {
        let fixture = bedrock_provider_fixture("us-east-1");
        let actual = fixture.transform_model_id("amazon.nova-pro-v1:0");
        let expected = "amazon.nova-pro-v1:0";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_transform_model_id_us_gov_region() {
        let fixture = bedrock_provider_fixture("us-gov-west-1");
        let actual = fixture.transform_model_id("anthropic.claude-3-sonnet");
        let expected = "anthropic.claude-3-sonnet";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_json_value_to_document_null() {
        let fixture = serde_json::Value::Null;
        let actual = json_value_to_document(fixture);
        let expected = aws_smithy_types::Document::Null;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_json_value_to_document_bool() {
        let fixture = serde_json::Value::Bool(true);
        let actual = json_value_to_document(fixture);
        let expected = aws_smithy_types::Document::Bool(true);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_json_value_to_document_number_int() {
        let fixture = serde_json::json!(42);
        let actual = json_value_to_document(fixture);
        let expected = aws_smithy_types::Document::Number(aws_smithy_types::Number::PosInt(42));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_json_value_to_document_number_float() {
        let value = 2.1;
        let fixture = serde_json::json!(value);
        let actual = json_value_to_document(fixture);
        let expected = aws_smithy_types::Document::Number(aws_smithy_types::Number::Float(value));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_json_value_to_document_string() {
        let fixture = serde_json::Value::String("hello".to_string());
        let actual = json_value_to_document(fixture);
        let expected = aws_smithy_types::Document::String("hello".to_string());
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_json_value_to_document_array() {
        let fixture = serde_json::json!([1, "two", true]);
        let actual = json_value_to_document(fixture);
        let expected = aws_smithy_types::Document::Array(vec![
            aws_smithy_types::Document::Number(aws_smithy_types::Number::PosInt(1)),
            aws_smithy_types::Document::String("two".to_string()),
            aws_smithy_types::Document::Bool(true),
        ]);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_json_value_to_document_object() {
        let fixture = serde_json::json!({"key": "value", "count": 10});
        let actual = json_value_to_document(fixture);

        let mut expected_map = std::collections::HashMap::new();
        expected_map.insert(
            "key".to_string(),
            aws_smithy_types::Document::String("value".to_string()),
        );
        expected_map.insert(
            "count".to_string(),
            aws_smithy_types::Document::Number(aws_smithy_types::Number::PosInt(10)),
        );
        let expected = aws_smithy_types::Document::Object(expected_map);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_json_value_to_document_nested() {
        let fixture = serde_json::json!({
            "outer": {
                "inner": [1, 2, 3],
                "flag": true
            }
        });
        let actual = json_value_to_document(fixture);

        let mut inner_map = std::collections::HashMap::new();
        inner_map.insert(
            "inner".to_string(),
            aws_smithy_types::Document::Array(vec![
                aws_smithy_types::Document::Number(aws_smithy_types::Number::PosInt(1)),
                aws_smithy_types::Document::Number(aws_smithy_types::Number::PosInt(2)),
                aws_smithy_types::Document::Number(aws_smithy_types::Number::PosInt(3)),
            ]),
        );
        inner_map.insert("flag".to_string(), aws_smithy_types::Document::Bool(true));

        let mut outer_map = std::collections::HashMap::new();
        outer_map.insert(
            "outer".to_string(),
            aws_smithy_types::Document::Object(inner_map),
        );
        let expected = aws_smithy_types::Document::Object(outer_map);

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_models_returns_hardcoded() {
        use forge_domain::{Model, ModelId, ModelSource};

        let mut fixture_provider = provider_fixture("token", None);
        let fixture_models = vec![
            Model {
                id: ModelId::from("claude-3-opus".to_string()),
                name: Some("Claude 3 Opus".to_string()),
                description: None,
                context_length: None,
                tools_supported: None,
                supports_parallel_tool_calls: None,
                supports_reasoning: None,
                input_modalities: vec![InputModality::Text],
            },
            Model {
                id: ModelId::from("claude-3-sonnet".to_string()),
                name: Some("Claude 3 Sonnet".to_string()),
                description: None,
                context_length: None,
                tools_supported: None,
                supports_parallel_tool_calls: None,
                supports_reasoning: None,
                input_modalities: vec![InputModality::Text],
            },
        ];
        fixture_provider.models = Some(ModelSource::Hardcoded(fixture_models.clone()));

        let bedrock = BedrockProvider {
            provider: fixture_provider,
            client: OnceCell::new(),
            region: "us-east-1".to_string(),
        };

        let actual = bedrock.models().await.unwrap();
        let expected = fixture_models;
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_models_returns_empty_when_no_models() {
        let fixture = provider_fixture("token", None);
        let bedrock = BedrockProvider {
            provider: fixture,
            client: OnceCell::new(),
            region: "us-east-1".to_string(),
        };

        let actual = bedrock.models().await.unwrap();
        let expected: Vec<Model> = vec![];
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_into_domain_content_block_delta_text() {
        use aws_sdk_bedrockruntime::types::{ContentBlockDelta, ConverseStreamOutput};
        use forge_domain::{ChatCompletionMessage, Content};

        let fixture = ConverseStreamOutput::ContentBlockDelta(
            aws_sdk_bedrockruntime::types::ContentBlockDeltaEvent::builder()
                .content_block_index(0)
                .delta(ContentBlockDelta::Text("hello".to_string()))
                .build()
                .unwrap(),
        );

        let actual = fixture.into_domain();
        let expected = ChatCompletionMessage::assistant(Content::part("hello"));

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_into_domain_content_block_start_tool_use() {
        use aws_sdk_bedrockruntime::types::{
            ContentBlockStart, ConverseStreamOutput, ToolUseBlockStart,
        };
        use forge_domain::{ChatCompletionMessage, Content, ToolCallId, ToolCallPart, ToolName};

        let fixture = ConverseStreamOutput::ContentBlockStart(
            aws_sdk_bedrockruntime::types::ContentBlockStartEvent::builder()
                .content_block_index(0)
                .start(ContentBlockStart::ToolUse(
                    ToolUseBlockStart::builder()
                        .tool_use_id("call_123")
                        .name("get_weather")
                        .build()
                        .unwrap(),
                ))
                .build()
                .unwrap(),
        );

        let actual = fixture.into_domain();
        let expected =
            ChatCompletionMessage::assistant(Content::part("")).add_tool_call(ToolCallPart {
                call_id: Some(ToolCallId::new("call_123")),
                name: Some(ToolName::new("get_weather")),
                arguments_part: String::new(),
                thought_signature: None,
            });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_into_domain_message_stop_end_turn() {
        use aws_sdk_bedrockruntime::types::{ConverseStreamOutput, StopReason};
        use forge_domain::{ChatCompletionMessage, Content, FinishReason};

        let fixture = ConverseStreamOutput::MessageStop(
            aws_sdk_bedrockruntime::types::MessageStopEvent::builder()
                .stop_reason(StopReason::EndTurn)
                .build()
                .unwrap(),
        );

        let actual = fixture.into_domain();
        let expected = ChatCompletionMessage::assistant(Content::part(""))
            .finish_reason_opt(Some(FinishReason::Stop));

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_into_domain_message_stop_tool_use() {
        use aws_sdk_bedrockruntime::types::{ConverseStreamOutput, StopReason};
        use forge_domain::{ChatCompletionMessage, Content, FinishReason};

        let fixture = ConverseStreamOutput::MessageStop(
            aws_sdk_bedrockruntime::types::MessageStopEvent::builder()
                .stop_reason(StopReason::ToolUse)
                .build()
                .unwrap(),
        );

        let actual = fixture.into_domain();
        let expected = ChatCompletionMessage::assistant(Content::part(""))
            .finish_reason_opt(Some(FinishReason::ToolCalls));

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_into_domain_metadata_with_usage() {
        use aws_sdk_bedrockruntime::types::ConverseStreamOutput;
        use forge_domain::{ChatCompletionMessage, Content, TokenCount};

        let fixture = ConverseStreamOutput::Metadata(
            aws_sdk_bedrockruntime::types::ConverseStreamMetadataEvent::builder()
                .usage(
                    aws_sdk_bedrockruntime::types::TokenUsage::builder()
                        .input_tokens(800)
                        .output_tokens(200)
                        .total_tokens(1000)
                        .cache_read_input_tokens(50)
                        .cache_write_input_tokens(30)
                        .build()
                        .unwrap(),
                )
                .build(),
        );

        let actual = fixture.into_domain();
        let expected =
            ChatCompletionMessage::assistant(Content::part("")).usage(forge_domain::Usage {
                prompt_tokens: TokenCount::Actual(800),
                completion_tokens: TokenCount::Actual(200),
                total_tokens: TokenCount::Actual(1000),
                cached_tokens: TokenCount::Actual(80), // 50 + 30
                ..Default::default()
            });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_from_domain_tool_choice_auto() {
        use aws_sdk_bedrockruntime::types::{AutoToolChoice, ToolChoice};

        let fixture = forge_domain::ToolChoice::Auto;
        let actual = ToolChoice::from_domain(fixture).unwrap();
        let expected = ToolChoice::Auto(AutoToolChoice::builder().build());

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_from_domain_tool_choice_required() {
        use aws_sdk_bedrockruntime::types::{AnyToolChoice, ToolChoice};

        let fixture = forge_domain::ToolChoice::Required;
        let actual = ToolChoice::from_domain(fixture).unwrap();
        let expected = ToolChoice::Any(AnyToolChoice::builder().build());

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_from_domain_tool_choice_call() {
        use aws_sdk_bedrockruntime::types::{SpecificToolChoice, ToolChoice};

        let fixture = forge_domain::ToolChoice::Call(forge_domain::ToolName::new("my_tool"));
        let actual = ToolChoice::from_domain(fixture).unwrap();
        let expected = ToolChoice::Tool(
            SpecificToolChoice::builder()
                .name("my_tool")
                .build()
                .unwrap(),
        );

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_from_domain_tool_definition() {
        use aws_sdk_bedrockruntime::types::Tool;
        use forge_domain::ToolDefinition;

        // In schemars 1.0, Schema wraps serde_json::Value
        let schema_value = serde_json::json!({});
        let schema = schemars::Schema::try_from(schema_value).unwrap();

        let fixture = ToolDefinition {
            name: forge_domain::ToolName::new("test_tool"),
            description: "A test tool".to_string(),
            input_schema: schema,
        };

        let actual = Tool::from_domain(fixture).unwrap();

        match actual {
            Tool::ToolSpec(spec) => {
                assert_eq!(spec.name(), "test_tool");
                assert_eq!(spec.description(), Some("A test tool"));
            }
            _ => panic!("Expected ToolSpec variant"),
        }
    }

    #[test]
    fn test_from_domain_context_message_text_user() {
        use aws_sdk_bedrockruntime::types::{ContentBlock, ConversationRole, Message};
        use forge_domain::{ContextMessage, Role, TextMessage};

        let fixture = ContextMessage::Text(TextMessage::new(Role::User, "Hello!"));

        let actual = Message::from_domain(fixture).unwrap();

        assert_eq!(actual.role(), &ConversationRole::User);
        let content = actual.content();
        assert_eq!(content.len(), 1);
        match &content[0] {
            ContentBlock::Text(text) => assert_eq!(text, "Hello!"),
            _ => panic!("Expected text content block"),
        }
    }

    #[test]
    fn test_from_domain_context_message_text_assistant() {
        use aws_sdk_bedrockruntime::types::{ContentBlock, ConversationRole, Message};
        use forge_domain::{ContextMessage, TextMessage};

        let fixture = ContextMessage::Text(TextMessage::assistant("Hi there!", None, None));

        let actual = Message::from_domain(fixture).unwrap();

        assert_eq!(actual.role(), &ConversationRole::Assistant);
        let content = actual.content();
        assert_eq!(content.len(), 1);
        match &content[0] {
            ContentBlock::Text(text) => assert_eq!(text, "Hi there!"),
            _ => panic!("Expected text content block"),
        }
    }

    #[test]
    fn test_from_domain_context_message_text_assistant_reasoning_uses_message_signature() {
        use aws_sdk_bedrockruntime::types::{
            ContentBlock, ConversationRole, Message, ReasoningContentBlock,
        };
        use forge_domain::{ContextMessage, ReasoningFull, TextMessage};
        use pretty_assertions::assert_eq;

        let fixture = ContextMessage::Text(
            TextMessage::assistant(
                "",
                Some(vec![
                    ReasoningFull::default().text(Some("Thinking...".to_string())),
                ]),
                None,
            )
            .thought_signature("sig_123"),
        );

        let actual = Message::from_domain(fixture).unwrap();
        let expected_signature = Some("sig_123");
        let expected_text = "Thinking...";

        assert_eq!(actual.role(), &ConversationRole::Assistant);
        let content = actual.content();
        assert_eq!(content.len(), 1);
        match &content[0] {
            ContentBlock::ReasoningContent(ReasoningContentBlock::ReasoningText(reasoning)) => {
                let actual_signature = reasoning.signature();
                let actual_text = reasoning.text();
                assert_eq!(actual_signature, expected_signature);
                assert_eq!(actual_text, expected_text);
            }
            _ => panic!("Expected reasoning content block"),
        }
    }

    #[test]
    fn test_from_domain_context_message_text_assistant_skips_unsigned_reasoning() {
        use aws_sdk_bedrockruntime::types::{
            ContentBlock, ConversationRole, Message, ReasoningContentBlock,
        };
        use forge_domain::{ContextMessage, ReasoningFull, TextMessage};
        use pretty_assertions::assert_eq;

        let fixture = ContextMessage::Text(TextMessage::assistant(
            "",
            Some(vec![
                ReasoningFull::default()
                    .text(Some("Signed reasoning".to_string()))
                    .signature(Some("sig_123".to_string())),
                ReasoningFull::default().text(Some("Unsigned duplicate reasoning".to_string())),
            ]),
            None,
        ));

        let actual = Message::from_domain(fixture).unwrap();

        assert_eq!(actual.role(), &ConversationRole::Assistant);
        let content = actual.content();
        assert_eq!(content.len(), 1);
        match &content[0] {
            ContentBlock::ReasoningContent(ReasoningContentBlock::ReasoningText(reasoning)) => {
                assert_eq!(reasoning.signature(), Some("sig_123"));
                assert_eq!(reasoning.text(), "Signed reasoning");
            }
            _ => panic!("Expected reasoning content block"),
        }
    }

    #[test]
    fn test_from_domain_context_message_tool_result() {
        use aws_sdk_bedrockruntime::types::{
            ContentBlock, ConversationRole, Message, ToolResultStatus,
        };
        use forge_domain::{ContextMessage, ToolCallId, ToolResult};

        let fixture = ContextMessage::Tool(
            ToolResult::new("test_tool")
                .call_id(ToolCallId::new("call_123"))
                .success("result data"),
        );

        let actual = Message::from_domain(fixture).unwrap();

        assert_eq!(actual.role(), &ConversationRole::User);
        let content = actual.content();
        assert_eq!(content.len(), 1);
        match &content[0] {
            ContentBlock::ToolResult(tool_result) => {
                assert_eq!(tool_result.tool_use_id(), "call_123");
                assert_eq!(tool_result.status(), Some(&ToolResultStatus::Success));
            }
            _ => panic!("Expected tool result content block"),
        }
    }

    #[test]
    fn test_from_domain_multiple_tool_results() {
        use aws_sdk_bedrockruntime::types::{ContentBlock, ConversationRole, Message};
        use forge_domain::{ContextMessage, ToolCallId, ToolResult};

        let fixture = vec![
            ContextMessage::Tool(
                ToolResult::new("tool_1")
                    .call_id(ToolCallId::new("call_1"))
                    .success("result 1"),
            ),
            ContextMessage::Tool(
                ToolResult::new("tool_2")
                    .call_id(ToolCallId::new("call_2"))
                    .success("result 2"),
            ),
        ];

        let actual = Message::from_domain(fixture).unwrap();

        assert_eq!(actual.role(), &ConversationRole::User);
        let content = actual.content();
        assert_eq!(content.len(), 2);

        match &content[0] {
            ContentBlock::ToolResult(tool_result) => {
                assert_eq!(tool_result.tool_use_id(), "call_1");
            }
            _ => panic!("Expected tool result content block"),
        }

        match &content[1] {
            ContentBlock::ToolResult(tool_result) => {
                assert_eq!(tool_result.tool_use_id(), "call_2");
            }
            _ => panic!("Expected tool result content block"),
        }
    }

    #[test]
    fn test_from_domain_context_with_system_messages() {
        use aws_sdk_bedrockruntime::operation::converse_stream::ConverseStreamInput;
        use forge_domain::{Context, ContextMessage, Role, TextMessage};

        let fixture = Context {
            conversation_id: None,
            initiator: None,
            messages: vec![
                ContextMessage::system("You are a helpful assistant").into(),
                ContextMessage::Text(TextMessage::new(Role::User, "Hello!")).into(),
            ],
            tools: vec![],
            tool_choice: None,
            temperature: None,
            top_p: None,
            top_k: None,
            max_tokens: None,
            reasoning: None,
            stream: None,
            response_format: None,
        };

        let actual = ConverseStreamInput::from_domain(fixture).unwrap();

        // System messages should be in system field
        let system = actual.system();
        assert_eq!(system.len(), 1);
        // User messages should be in messages field
        let messages = actual.messages();
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn test_from_domain_context_with_temperature() {
        use aws_sdk_bedrockruntime::operation::converse_stream::ConverseStreamInput;
        use forge_domain::{Context, Temperature};

        let fixture = Context {
            conversation_id: None,
            initiator: None,
            messages: vec![],
            tools: vec![],
            tool_choice: None,
            temperature: Some(Temperature::new(0.7).unwrap()),
            top_p: None,
            top_k: None,
            max_tokens: None,
            reasoning: None,
            stream: None,
            response_format: None,
        };

        let actual = ConverseStreamInput::from_domain(fixture).unwrap();

        assert!(actual.inference_config().is_some());
        assert_eq!(actual.inference_config().unwrap().temperature(), Some(0.7));
    }

    #[test]
    fn test_from_domain_context_with_reasoning_adjusts_top_p() {
        use aws_sdk_bedrockruntime::operation::converse_stream::ConverseStreamInput;
        use forge_domain::{Context, ReasoningConfig, TopP};

        let fixture = Context {
            conversation_id: None,
            initiator: None,
            messages: vec![],
            tools: vec![],
            tool_choice: None,
            temperature: None,
            top_p: Some(TopP::new(0.5).unwrap()), // Below 0.95
            top_k: None,
            max_tokens: None,
            reasoning: Some(ReasoningConfig {
                effort: None,
                max_tokens: Some(2000),
                exclude: None,
                enabled: Some(true),
            }),
            stream: None,
            response_format: None,
        };

        let actual = ConverseStreamInput::from_domain(fixture).unwrap();

        // When reasoning is enabled, top_p should be adjusted to at least 0.95
        assert!(actual.inference_config().is_some());
        let top_p = actual.inference_config().unwrap().top_p();
        assert!(top_p.is_some());
        assert!(top_p.unwrap() >= 0.95);
    }

    #[test]
    fn test_from_domain_context_with_reasoning_enabled() {
        use aws_sdk_bedrockruntime::operation::converse_stream::ConverseStreamInput;
        use forge_domain::{Context, ReasoningConfig};

        let fixture = Context {
            conversation_id: None,
            initiator: None,
            messages: vec![],
            tools: vec![],
            tool_choice: None,
            temperature: None,
            top_p: None,
            top_k: None,
            max_tokens: None,
            reasoning: Some(ReasoningConfig {
                effort: None,
                max_tokens: Some(3000),
                exclude: None,
                enabled: Some(true),
            }),
            stream: None,
            response_format: None,
        };

        let actual = ConverseStreamInput::from_domain(fixture).unwrap();

        // Should have additional model request fields for reasoning
        assert!(actual.additional_model_request_fields().is_some());
    }

    #[test]
    fn test_json_value_to_document_empty_object() {
        let fixture = serde_json::json!({});
        let actual = json_value_to_document(fixture);

        match actual {
            aws_smithy_types::Document::Object(map) => {
                assert!(map.is_empty());
            }
            _ => panic!("Expected object document"),
        }
    }

    #[test]
    fn test_json_value_to_document_empty_array() {
        let fixture = serde_json::json!([]);
        let actual = json_value_to_document(fixture);

        match actual {
            aws_smithy_types::Document::Array(arr) => {
                assert!(arr.is_empty());
            }
            _ => panic!("Expected array document"),
        }
    }
}
