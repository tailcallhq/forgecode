use anyhow::Context as _;
use tokio_stream::StreamExt;

use crate::reasoning::{Reasoning, ReasoningFull};
use crate::{
    ArcSender, ChatCompletionMessage, ChatCompletionMessageFull, ChatResponse, ChatResponseContent,
    ToolCallFull, ToolCallPart, Usage,
};

/// Extension trait for ResultStream to provide additional functionality
#[async_trait::async_trait]
pub trait ResultStreamExt<E> {
    /// Collects all messages from the stream into a single
    /// ChatCompletionMessageFull
    ///
    /// # Arguments
    /// * `should_interrupt_for_xml` - Whether to interrupt the stream when XML
    ///   tool calls are detected
    ///
    /// # Returns
    /// A ChatCompletionMessageFull containing the aggregated content, tool
    /// calls, and usage information
    async fn into_full(
        self,
        should_interrupt_for_xml: bool,
    ) -> Result<ChatCompletionMessageFull, E>;

    /// Collects all messages from the stream into a single
    /// ChatCompletionMessageFull while streaming content deltas to the sender.
    ///
    /// # Arguments
    /// * `should_interrupt_for_xml` - Whether to interrupt the stream when XML
    ///   tool calls are detected
    /// * `sender` - Optional sender to stream content and reasoning deltas to
    ///
    /// # Returns
    /// A ChatCompletionMessageFull containing the aggregated content, tool
    /// calls, and usage information
    async fn into_full_streaming(
        self,
        should_interrupt_for_xml: bool,
        sender: Option<ArcSender>,
    ) -> Result<ChatCompletionMessageFull, E>;
}

#[async_trait::async_trait]
impl ResultStreamExt<anyhow::Error> for crate::BoxStream<ChatCompletionMessage, anyhow::Error> {
    async fn into_full(
        self,
        should_interrupt_for_xml: bool,
    ) -> anyhow::Result<ChatCompletionMessageFull> {
        self.into_full_streaming(should_interrupt_for_xml, None)
            .await
    }

    async fn into_full_streaming(
        mut self,
        should_interrupt_for_xml: bool,
        sender: Option<ArcSender>,
    ) -> anyhow::Result<ChatCompletionMessageFull> {
        let mut messages = Vec::new();
        let mut usage: Usage = Default::default();
        let mut content = String::new();
        let mut xml_tool_calls = None;
        let mut tool_interrupted = false;

        while let Some(message) = self.next().await {
            let message =
                anyhow::Ok(message?).with_context(|| "Failed to process message stream")?;
            // Process usage information
            // - For Anthropic-style streaming: input tokens in MessageStart, output tokens
            //   in MessageDelta (values are CUMULATIVE, not incremental)
            //   ref: https://platform.claude.com/docs/en/build-with-claude/streaming#event-types
            // - For OpenAI-style streaming: all tokens in the final chunk
            // - For GLM-style: may send complete usage in every chunk (need to replace, not
            //   accumulate)
            // - For Google-style: cumulative usage in every chunk
            // - Cost-only events: have 0 tokens but a cost value
            if let Some(current_usage) = message.usage.as_ref() {
                // If current usage has both prompt and completion tokens, it's a "complete"
                // usage. In this case, replace instead of merge (handles GLM-style streaming
                // where every chunk has full usage).
                let is_complete_usage =
                    *current_usage.prompt_tokens > 0 && *current_usage.completion_tokens > 0;

                // Cost-only events have 0 tokens but a cost value
                let is_cost_only = *current_usage.prompt_tokens == 0
                    && *current_usage.completion_tokens == 0
                    && current_usage.cost.is_some();

                if is_complete_usage {
                    // Replace with the latest complete usage, but preserve cost if already set
                    let existing_cost = usage.cost;
                    usage = *current_usage;
                    if usage.cost.is_none() && existing_cost.is_some() {
                        usage.cost = existing_cost;
                    }
                } else if is_cost_only {
                    // Accumulate only the cost to the existing usage
                    usage.cost = match (usage.cost, current_usage.cost) {
                        (Some(a), Some(b)) => Some(a + b),
                        (Some(a), None) => Some(a),
                        (None, Some(b)) => Some(b),
                        (None, None) => None,
                    };
                } else {
                    // Merge partial usage using "max" strategy. This correctly handles
                    // providers like Anthropic where usage values are CUMULATIVE across
                    // events (message_start has input tokens, message_delta has the
                    // total output tokens). Using max instead of sum prevents
                    // double-counting when message_start includes output_tokens=1.
                    usage = usage.merge(current_usage);
                }
            }

            if !tool_interrupted {
                messages.push(message.clone());

                // Stream content delta if sender is available
                if let Some(ref sender) = sender {
                    if let Some(reasoning_part) = message.reasoning.as_ref() {
                        let delta = reasoning_part.as_str();
                        if !delta.is_empty() {
                            // Ignore send errors - the receiver may have been dropped
                            let _ = sender
                                .send(Ok(ChatResponse::TaskReasoning {
                                    content: delta.to_string(),
                                }))
                                .await;
                        }
                    }

                    if let Some(content_part) = message.content.as_ref() {
                        let delta = content_part.as_str();
                        if !delta.is_empty() {
                            // Ignore send errors - the receiver may have been dropped
                            let _ = sender
                                .send(Ok(ChatResponse::TaskMessage {
                                    content: ChatResponseContent::Markdown {
                                        text: delta.to_string(),
                                        partial: true,
                                    },
                                }))
                                .await;
                        }
                    }
                }

                // Process content
                if let Some(content_part) = message.content.as_ref() {
                    content.push_str(content_part.as_str());

                    // Check for XML tool calls in the content, but only interrupt if flag is set
                    if should_interrupt_for_xml {
                        // Use match instead of ? to avoid propagating errors
                        if let Some(tool_call) = ToolCallFull::try_from_xml(&content)
                            .ok()
                            .into_iter()
                            .flatten()
                            .next()
                        {
                            xml_tool_calls = Some(tool_call);
                            tool_interrupted = true;
                        }
                    }
                }
            }
        }

        // Get the full content from all messages
        let mut content = messages
            .iter()
            .flat_map(|m| m.content.iter())
            .map(|content| content.as_str())
            .collect::<Vec<_>>()
            .join("");

        // Collect reasoning tokens from all messages
        let reasoning = messages
            .iter()
            .flat_map(|m| m.reasoning.iter())
            .map(|content| content.as_str())
            .collect::<Vec<_>>()
            .join("");

        #[allow(clippy::collapsible_if)]
        if tool_interrupted && !content.trim().ends_with("</forge_tool_call>") {
            if let Some((i, right)) = content.rmatch_indices("</forge_tool_call>").next() {
                content.truncate(i + right.len());

                // Add a comment for the assistant to signal interruption
                content.push('\n');
                content.push_str("<forge_feedback>");
                content.push_str(
                    "Response interrupted by tool result. Use only one tool at the end of the message",
                );
                content.push_str("</forge_feedback>");
            }
        }

        // Extract all tool calls in a fully declarative way with combined sources
        // Start with complete tool calls (for non-streaming mode)
        let initial_tool_calls: Vec<ToolCallFull> = messages
            .iter()
            .flat_map(|message| &message.tool_calls)
            .filter_map(|tool_call| tool_call.as_full().cloned())
            .collect();

        // Get partial tool calls
        let tool_call_parts: Vec<ToolCallPart> = messages
            .iter()
            .flat_map(|message| &message.tool_calls)
            .filter_map(|tool_call| tool_call.as_partial().cloned())
            .collect();

        // Process partial tool calls
        // Convert parse failures to retryable errors so they can be retried by asking
        // LLM to try again
        let partial_tool_calls = ToolCallFull::try_from_parts(&tool_call_parts)
            .with_context(|| "Failed to parse tool call".to_string())
            .map_err(crate::Error::Retryable)?;

        // Combine all sources of tool calls
        let tool_calls: Vec<ToolCallFull> = initial_tool_calls
            .into_iter()
            .chain(partial_tool_calls)
            .chain(xml_tool_calls)
            .collect();

        // Collect reasoning details from all messages
        let initial_reasoning_details = messages
            .iter()
            .filter_map(|message| message.reasoning_details.as_ref())
            .flat_map(|details| details.iter().filter_map(|d| d.as_full().cloned()))
            .flatten()
            .collect::<Vec<_>>();
        let partial_reasoning_details = messages
            .iter()
            .filter_map(|message| message.reasoning_details.as_ref())
            .flat_map(|details| details.iter().filter_map(|d| d.as_partial().cloned()))
            .collect::<Vec<_>>();
        let total_reasoning_details: Vec<ReasoningFull> = initial_reasoning_details
            .into_iter()
            .chain(Reasoning::from_parts(partial_reasoning_details))
            .collect();

        // Get the finish reason from the last message that has one
        let finish_reason = messages
            .iter()
            .rev()
            .find_map(|message| message.finish_reason.clone());

        // Get thought signature from the last message that has one
        let thought_signature = messages
            .iter()
            .rev()
            .find_map(|message| message.thought_signature.clone());

        // Get phase from the last message that has one
        let phase = messages.iter().rev().find_map(|message| message.phase);

        // Check for empty completion - map to retryable error for retry
        if content.trim().is_empty()
            && tool_calls.is_empty()
            && finish_reason.is_none()
            && thought_signature.is_none()
        {
            return Err(crate::Error::EmptyCompletion.into_retryable().into());
        }

        Ok(ChatCompletionMessageFull {
            content,
            thought_signature,
            tool_calls,
            usage,
            reasoning: (!reasoning.is_empty()).then_some(reasoning),
            reasoning_details: (!total_reasoning_details.is_empty())
                .then_some(total_reasoning_details),
            finish_reason,
            phase,
        })
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::{
        BoxStream, Content, FinishReason, TokenCount, ToolCall, ToolCallArguments, ToolCallId,
        ToolName,
    };

    #[tokio::test]
    async fn test_into_full_basic() {
        // Fixture: Create a stream of messages
        // OpenAI-style: usage only in the final chunk
        let messages = vec![
            Ok(ChatCompletionMessage::default().content(Content::part("Hello "))),
            Ok(ChatCompletionMessage::default()
                .content(Content::part("world!"))
                .usage(Usage {
                    prompt_tokens: TokenCount::Actual(10),
                    completion_tokens: TokenCount::Actual(5),
                    total_tokens: TokenCount::Actual(15),
                    cached_tokens: TokenCount::Actual(0),
                    cost: None,
                })),
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: Combined content and usage from final chunk
        let expected = ChatCompletionMessageFull {
            content: "Hello world!".to_string(),
            tool_calls: vec![],
            thought_signature: None,
            usage: Usage {
                prompt_tokens: TokenCount::Actual(10),    // From final chunk
                completion_tokens: TokenCount::Actual(5), // From final chunk
                total_tokens: TokenCount::Actual(15),     // From final chunk
                cached_tokens: TokenCount::Actual(0),
                cost: None,
            },
            reasoning: None,
            reasoning_details: None,
            finish_reason: None,
            phase: None,
        };

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_into_full_glm_style_usage_replacement() {
        // Fixture: Simulate GLM-style streaming where complete usage is sent in every
        // chunk This tests that we replace instead of accumulate to avoid
        // multiplying tokens
        let messages = vec![
            Ok(ChatCompletionMessage::default()
                .content(Content::part("Hello "))
                .usage(Usage {
                    prompt_tokens: TokenCount::Actual(100),
                    completion_tokens: TokenCount::Actual(5),
                    total_tokens: TokenCount::Actual(105),
                    cached_tokens: TokenCount::Actual(0),
                    cost: None,
                })),
            Ok(ChatCompletionMessage::default()
                .content(Content::part("world!"))
                .usage(Usage {
                    prompt_tokens: TokenCount::Actual(100),
                    completion_tokens: TokenCount::Actual(10),
                    total_tokens: TokenCount::Actual(110),
                    cached_tokens: TokenCount::Actual(0),
                    cost: None,
                })),
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: Usage should be from the last chunk, NOT accumulated
        // (accumulating would give prompt_tokens=200, which is wrong)
        let expected = ChatCompletionMessageFull {
            content: "Hello world!".to_string(),
            tool_calls: vec![],
            thought_signature: None,
            usage: Usage {
                prompt_tokens: TokenCount::Actual(100), // From last chunk, NOT 200
                completion_tokens: TokenCount::Actual(10), // From last chunk
                total_tokens: TokenCount::Actual(110),  // From last chunk, NOT 215
                cached_tokens: TokenCount::Actual(0),
                cost: None,
            },
            reasoning: None,
            reasoning_details: None,
            finish_reason: None,
            phase: None,
        };

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_into_full_cost_only_event_adds_cost_to_usage() {
        // Fixture: Simulate GLM-style streaming with a separate cost event at the end
        let messages = vec![
            // Content with complete usage
            Ok(ChatCompletionMessage::default()
                .content(Content::part("Hello world!"))
                .usage(Usage {
                    prompt_tokens: TokenCount::Actual(100),
                    completion_tokens: TokenCount::Actual(10),
                    total_tokens: TokenCount::Actual(110),
                    cached_tokens: TokenCount::Actual(0),
                    cost: None,
                })),
            // Cost-only event (0 tokens but has cost)
            Ok(ChatCompletionMessage::default().usage(Usage {
                prompt_tokens: TokenCount::Actual(0),
                completion_tokens: TokenCount::Actual(0),
                total_tokens: TokenCount::Actual(0),
                cached_tokens: TokenCount::Actual(0),
                cost: Some(0.005),
            })),
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: Usage should have tokens from first chunk, cost from cost-only
        // event
        let expected = ChatCompletionMessageFull {
            content: "Hello world!".to_string(),
            tool_calls: vec![],
            thought_signature: None,
            usage: Usage {
                prompt_tokens: TokenCount::Actual(100),
                completion_tokens: TokenCount::Actual(10),
                total_tokens: TokenCount::Actual(110),
                cached_tokens: TokenCount::Actual(0),
                cost: Some(0.005), // From cost-only event
            },
            reasoning: None,
            reasoning_details: None,
            finish_reason: None,
            phase: None,
        };

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_into_full_cost_preserved_when_complete_usage_arrives_after_cost_only() {
        // Fixture: Cost-only event arrives BEFORE the complete usage event
        // (Graphite bug report scenario)
        let messages = vec![
            // Cost-only event arrives first
            Ok(ChatCompletionMessage::default().usage(Usage {
                prompt_tokens: TokenCount::Actual(0),
                completion_tokens: TokenCount::Actual(0),
                total_tokens: TokenCount::Actual(0),
                cached_tokens: TokenCount::Actual(0),
                cost: Some(0.005),
            })),
            // Complete usage event arrives after (without cost)
            Ok(ChatCompletionMessage::default()
                .content(Content::part("Hello world!"))
                .usage(Usage {
                    prompt_tokens: TokenCount::Actual(100),
                    completion_tokens: TokenCount::Actual(10),
                    total_tokens: TokenCount::Actual(110),
                    cached_tokens: TokenCount::Actual(0),
                    cost: None,
                })),
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: Cost from cost-only event should NOT be lost when complete usage
        // replaces it
        let expected = ChatCompletionMessageFull {
            content: "Hello world!".to_string(),
            tool_calls: vec![],
            thought_signature: None,
            usage: Usage {
                prompt_tokens: TokenCount::Actual(100),
                completion_tokens: TokenCount::Actual(10),
                total_tokens: TokenCount::Actual(110),
                cached_tokens: TokenCount::Actual(0),
                cost: Some(0.005), // Preserved from cost-only event
            },
            reasoning: None,
            reasoning_details: None,
            finish_reason: None,
            phase: None,
        };

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_into_full_anthropic_streaming_usage_merge() {
        // Fixture: Simulate Anthropic streaming pattern where message_start has
        // output_tokens=1 (the common case) and message_delta has the cumulative total.
        // This tests that merge (max) is used instead of accumulate (sum) to prevent
        // double-counting.
        let messages = vec![
            // MessageStart with input token usage AND output_tokens=1
            Ok(ChatCompletionMessage::default().usage(Usage {
                prompt_tokens: TokenCount::Actual(1000),
                completion_tokens: TokenCount::Actual(1),
                total_tokens: TokenCount::Actual(1001),
                cached_tokens: TokenCount::Actual(300),
                cost: None,
            })),
            // Content deltas
            Ok(ChatCompletionMessage::default().content(Content::part("Hello "))),
            Ok(ChatCompletionMessage::default().content(Content::part("world!"))),
            // MessageDelta with cumulative output token usage
            Ok(ChatCompletionMessage::default()
                .usage(Usage {
                    prompt_tokens: TokenCount::Actual(0),
                    completion_tokens: TokenCount::Actual(50),
                    total_tokens: TokenCount::Actual(50),
                    cached_tokens: TokenCount::Actual(0),
                    cost: None,
                })
                .finish_reason(FinishReason::Stop)),
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: Usage should use max (merge) not sum (accumulate).
        // message_start has completion_tokens=1 and prompt_tokens=1000, so
        // is_complete_usage=true -> replace: usage = {1000, 1, 1001, 300}
        // message_delta has prompt=0, completion=50 -> is_complete_usage=false ->
        // merge:   prompt = max(1000, 0) = 1000
        //   completion = max(1, 50) = 50 (NOT 1+50=51)
        //   total = max(1001, 50) = 1001
        //   cached = max(300, 0) = 300
        let expected = ChatCompletionMessageFull {
            content: "Hello world!".to_string(),
            tool_calls: vec![],
            thought_signature: None,
            usage: Usage {
                prompt_tokens: TokenCount::Actual(1000),
                completion_tokens: TokenCount::Actual(50), // max(1, 50) = 50, NOT 1+50=51
                total_tokens: TokenCount::Actual(1001),
                cached_tokens: TokenCount::Actual(300),
                cost: None,
            },
            reasoning: None,
            reasoning_details: None,
            finish_reason: Some(FinishReason::Stop),
            phase: None,
        };

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_into_full_anthropic_streaming_usage_merge_zero_output() {
        // Fixture: Simulate Anthropic/Vertex AI Anthropic streaming pattern
        // where message_start has output_tokens=0 (Vertex AI pattern).
        // MessageStart event has input tokens, MessageDelta has output tokens
        let messages = vec![
            // MessageStart with input token usage
            Ok(ChatCompletionMessage::default().usage(Usage {
                prompt_tokens: TokenCount::Actual(1000),
                completion_tokens: TokenCount::Actual(0),
                total_tokens: TokenCount::Actual(1000),
                cached_tokens: TokenCount::Actual(300),
                cost: None,
            })),
            // Content deltas
            Ok(ChatCompletionMessage::default().content(Content::part("Hello "))),
            Ok(ChatCompletionMessage::default().content(Content::part("world!"))),
            // MessageDelta with output token usage
            Ok(ChatCompletionMessage::default()
                .usage(Usage {
                    prompt_tokens: TokenCount::Actual(0),
                    completion_tokens: TokenCount::Actual(50),
                    total_tokens: TokenCount::Actual(50),
                    cached_tokens: TokenCount::Actual(0),
                    cost: None,
                })
                .finish_reason(FinishReason::Stop)),
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: Usage should be merged from both MessageStart and MessageDelta
        let expected = ChatCompletionMessageFull {
            content: "Hello world!".to_string(),
            tool_calls: vec![],
            thought_signature: None,
            usage: Usage {
                prompt_tokens: TokenCount::Actual(1000), // From MessageStart
                completion_tokens: TokenCount::Actual(50), // From MessageDelta
                total_tokens: TokenCount::Actual(1000),  // max(1000, 50) = 1000
                cached_tokens: TokenCount::Actual(300),  // From MessageStart
                cost: None,
            },
            reasoning: None,
            reasoning_details: None,
            finish_reason: Some(FinishReason::Stop),
            phase: None,
        };

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_into_full_streaming_sends_deltas() {
        // Fixture: Create a stream of messages
        let messages = vec![
            Ok(ChatCompletionMessage::default().content(Content::part("Hello "))),
            Ok(ChatCompletionMessage::default().content(Content::part("world!"))),
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Create a channel to receive deltas
        let (tx, mut rx) = tokio::sync::mpsc::channel::<anyhow::Result<ChatResponse>>(10);

        // Actual: Convert stream to full message with streaming
        let actual = result_stream
            .into_full_streaming(false, Some(tx))
            .await
            .unwrap();

        // Collect all deltas
        let mut deltas = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            deltas.push(msg.unwrap());
        }

        // Expected: Two deltas were sent as TaskMessage with Markdown content
        assert_eq!(deltas.len(), 2);
        assert!(matches!(
            &deltas[0],
            ChatResponse::TaskMessage { content: ChatResponseContent::Markdown { text, partial: true }, .. } if text == "Hello "
        ));
        assert!(matches!(
            &deltas[1],
            ChatResponse::TaskMessage { content: ChatResponseContent::Markdown { text, partial: true }, .. } if text == "world!"
        ));

        // Expected: Full content is still correct
        assert_eq!(actual.content, "Hello world!");
    }

    #[tokio::test]
    async fn test_into_full_streaming_sends_reasoning_deltas() {
        // Fixture: Create a stream of messages with reasoning
        let messages = vec![
            Ok(ChatCompletionMessage::default()
                .content(Content::part("Answer: "))
                .reasoning(Content::part("Let me think..."))),
            Ok(ChatCompletionMessage::default()
                .content(Content::part("42"))
                .reasoning(Content::part(" about this."))),
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Create a channel to receive deltas
        let (tx, mut rx) = tokio::sync::mpsc::channel::<anyhow::Result<ChatResponse>>(10);

        // Actual: Convert stream to full message with streaming
        let actual = result_stream
            .into_full_streaming(false, Some(tx))
            .await
            .unwrap();

        // Collect all deltas
        let mut content_deltas = Vec::new();
        let mut reasoning_deltas = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            match msg.unwrap() {
                ChatResponse::TaskMessage {
                    content: ChatResponseContent::Markdown { text, partial: true },
                    ..
                } => content_deltas.push(text),
                ChatResponse::TaskReasoning { content } => reasoning_deltas.push(content),
                _ => {}
            }
        }

        // Expected: Two content deltas and two reasoning deltas
        assert_eq!(content_deltas.len(), 2);
        assert_eq!(reasoning_deltas.len(), 2);
        assert_eq!(content_deltas, vec!["Answer: ", "42"]);
        assert_eq!(reasoning_deltas, vec!["Let me think...", " about this."]);

        // Expected: Full content and reasoning are correct
        assert_eq!(actual.content, "Answer: 42");
        assert_eq!(
            actual.reasoning,
            Some("Let me think... about this.".to_string())
        );
    }

    #[tokio::test]
    async fn test_into_full_with_tool_calls() {
        // Fixture: Create a stream with tool calls
        let tool_call = ToolCallFull {
            name: ToolName::new("test_tool"),
            call_id: Some(ToolCallId::new("call_123")),
            arguments: serde_json::json!("test_arg").into(),
            thought_signature: None,
        };

        let messages = vec![Ok(ChatCompletionMessage::default()
            .content(Content::part("Processing..."))
            .add_tool_call(ToolCall::Full(tool_call.clone())))];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: Content and tool calls
        let expected = ChatCompletionMessageFull {
            content: "Processing...".to_string(),
            tool_calls: vec![tool_call],
            thought_signature: None,
            usage: Usage::default(),
            reasoning: None,
            reasoning_details: None,
            finish_reason: None,
            phase: None,
        };

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_into_full_with_tool_call_parse_failure_creates_retryable_error() {
        use crate::{ToolCallId, ToolCallPart, ToolName};

        // Fixture: Create a stream with invalid tool call JSON
        let invalid_tool_call_part = ToolCallPart {
            call_id: Some(ToolCallId::new("call_123")),
            name: Some(ToolName::new("test_tool")),
            arguments_part: "invalid json {".to_string(), // Invalid JSON
            thought_signature: None,
        };

        let messages = vec![Ok(ChatCompletionMessage::default()
            .content(Content::part("Processing..."))
            .add_tool_call(ToolCall::Part(invalid_tool_call_part)))];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await;

        // Expected: Should not fail with invalid tool calls
        assert!(actual.is_ok());
        let actual = actual.unwrap();
        let expected = ToolCallFull {
            name: ToolName::new("test_tool"),
            call_id: Some(ToolCallId::new("call_123")),
            arguments: ToolCallArguments::from_json("invalid json {"),
            thought_signature: None,
        };
        assert_eq!(actual.tool_calls[0], expected);
    }

    #[tokio::test]
    async fn test_into_full_with_reasoning() {
        // Fixture: Create a stream with reasoning content across multiple messages
        let messages = vec![
            Ok(ChatCompletionMessage::default()
                .content(Content::part("Hello "))
                .reasoning(Content::part("First reasoning: "))),
            Ok(ChatCompletionMessage::default()
                .content(Content::part("world!"))
                .reasoning(Content::part("thinking deeply about this..."))),
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: Reasoning should be aggregated from all messages
        let expected = ChatCompletionMessageFull {
            content: "Hello world!".to_string(),
            tool_calls: vec![],
            thought_signature: None,
            usage: Usage::default(),
            reasoning: Some("First reasoning: thinking deeply about this...".to_string()),
            reasoning_details: None,
            finish_reason: None,
            phase: None,
        };

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_into_full_with_reasoning_details() {
        use crate::reasoning::{Reasoning, ReasoningFull};

        // Fixture: Create a stream with reasoning details
        let reasoning_full = vec![ReasoningFull {
            text: Some("Deep thought process".to_string()),
            signature: Some("signature1".to_string()),
            ..Default::default()
        }];

        let reasoning_part = crate::reasoning::ReasoningPart {
            text: Some("Partial reasoning".to_string()),
            signature: Some("signature2".to_string()),
            ..Default::default()
        };

        let messages = vec![
            Ok(ChatCompletionMessage::default()
                .content(Content::part("Processing..."))
                .add_reasoning_detail(Reasoning::Full(reasoning_full.clone()))),
            Ok(ChatCompletionMessage::default()
                .content(Content::part(" complete"))
                .add_reasoning_detail(Reasoning::Part(vec![reasoning_part]))),
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: Reasoning details should be collected from all messages
        let expected_reasoning_details = vec![
            reasoning_full[0].clone(),
            ReasoningFull {
                text: Some("Partial reasoning".to_string()),
                signature: Some("signature2".to_string()),
                ..Default::default()
            },
        ];

        let expected = ChatCompletionMessageFull {
            content: "Processing... complete".to_string(),
            tool_calls: vec![],
            thought_signature: None,
            usage: Usage::default(),
            reasoning: None,
            reasoning_details: Some(expected_reasoning_details),
            finish_reason: None,
            phase: None,
        };

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_into_full_with_empty_reasoning() {
        // Fixture: Create a stream with empty reasoning
        let messages = vec![
            Ok(ChatCompletionMessage::default().content(Content::part("Hello"))),
            Ok(ChatCompletionMessage::default()
                .content(Content::part(" world"))
                .reasoning(Content::part(""))), // Empty reasoning
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: Empty reasoning should result in None
        let expected = ChatCompletionMessageFull {
            content: "Hello world".to_string(),
            tool_calls: vec![],
            thought_signature: None,
            usage: Usage::default(),
            reasoning: None, // Empty reasoning should be None
            reasoning_details: None,
            finish_reason: None,
            phase: None,
        };

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_into_full_xml_tool_call_interruption_captures_final_usage() {
        let xml_content = r#"<forge_tool_call>
{"name": "test_tool", "arguments": {"arg": "value"}}
</forge_tool_call>"#;

        let messages = vec![
            Ok(ChatCompletionMessage::default().content(Content::part(&xml_content[0..30]))),
            Ok(ChatCompletionMessage::default().content(Content::part(&xml_content[30..]))),
            // These messages come after tool interruption but contain usage updates
            Ok(ChatCompletionMessage::default().content(Content::part(" ignored content"))),
            // Final message with the actual usage - this is always sent last
            Ok(ChatCompletionMessage::default().usage(Usage {
                prompt_tokens: TokenCount::Actual(5),
                completion_tokens: TokenCount::Actual(15),
                total_tokens: TokenCount::Actual(20),
                cached_tokens: TokenCount::Actual(0),
                cost: None,
            })),
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message with XML interruption enabled
        let actual = result_stream.into_full(true).await.unwrap();

        // Expected: Should contain the XML tool call and final usage from last message
        let expected_final_usage = Usage {
            prompt_tokens: TokenCount::Actual(5),
            completion_tokens: TokenCount::Actual(15),
            total_tokens: TokenCount::Actual(20),
            cached_tokens: TokenCount::Actual(0),
            cost: None,
        };
        assert_eq!(actual.usage, expected_final_usage);
        assert_eq!(actual.tool_calls.len(), 1);
        assert_eq!(actual.tool_calls[0].name.as_str(), "test_tool");
        assert_eq!(actual.content, xml_content);
    }

    #[tokio::test]
    async fn test_into_full_xml_tool_call_no_interruption_when_disabled() {
        // Fixture: Create a stream with XML tool call content but interruption disabled
        let xml_content = r#"<forge_tool_call>
{"name": "test_tool", "arguments": {"arg": "value"}}
</forge_tool_call>"#;

        let messages = vec![
            Ok(ChatCompletionMessage::default().content(Content::part(xml_content))),
            Ok(ChatCompletionMessage::default()
                .content(Content::part(" and more content"))
                .usage(Usage {
                    prompt_tokens: TokenCount::Actual(5),
                    completion_tokens: TokenCount::Actual(15),
                    total_tokens: TokenCount::Actual(20),
                    cached_tokens: TokenCount::Actual(0),
                    cost: None,
                })),
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message with XML interruption disabled
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: Should process all content without interruption
        let expected = ChatCompletionMessageFull {
            content: format!("{xml_content} and more content"),
            tool_calls: vec![], /* No XML tool calls should be extracted when interruption is
                                 * disabled */
            thought_signature: None,
            usage: Usage {
                prompt_tokens: TokenCount::Actual(5),
                completion_tokens: TokenCount::Actual(15),
                total_tokens: TokenCount::Actual(20),
                cached_tokens: TokenCount::Actual(0),
                cost: None,
            },
            reasoning: None,
            reasoning_details: None,
            finish_reason: None,
            phase: None,
        };

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_into_full_usage_always_from_last_message_even_without_interruption() {
        // Fixture: Create a stream where usage progresses through multiple messages
        let messages = vec![
            Ok(ChatCompletionMessage::default().content(Content::part("Starting"))),
            Ok(ChatCompletionMessage::default().content(Content::part(" processing"))),
            Ok(ChatCompletionMessage::default().content(Content::part(" complete"))),
            Ok(ChatCompletionMessage::default().usage(Usage {
                prompt_tokens: TokenCount::Actual(5),
                completion_tokens: TokenCount::Actual(15),
                total_tokens: TokenCount::Actual(20),
                cached_tokens: TokenCount::Actual(0),
                cost: None,
            })),
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: Usage should be from the last message (even if it has no content)
        let expected = ChatCompletionMessageFull {
            content: "Starting processing complete".to_string(),
            tool_calls: vec![],
            thought_signature: None,
            usage: Usage {
                prompt_tokens: TokenCount::Actual(5),
                completion_tokens: TokenCount::Actual(15),
                total_tokens: TokenCount::Actual(20),
                cached_tokens: TokenCount::Actual(0),
                cost: None,
            },
            reasoning: None,
            reasoning_details: None,
            finish_reason: None,
            phase: None,
        };

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_into_full_with_finish_reason() {
        use crate::FinishReason;

        // Fixture: Create a stream with multiple messages, some with finish reasons
        let messages = vec![
            Ok(ChatCompletionMessage::default()
                .content(Content::part("Processing..."))
                .finish_reason_opt(Some(FinishReason::Length))), /* This finish reason should be
                                                                  * overridden */
            Ok(ChatCompletionMessage::default()
                .content(Content::part(" continue"))
                .finish_reason_opt(None)), // No finish reason
            Ok(ChatCompletionMessage::default()
                .content(Content::part(" done"))
                .finish_reason_opt(Some(FinishReason::Stop))), /* This should be the final
                                                                * finish reason */
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: Should use the last finish reason from the stream
        let expected = ChatCompletionMessageFull {
            content: "Processing... continue done".to_string(),
            tool_calls: vec![],
            thought_signature: None,
            usage: Usage::default(),
            reasoning: None,
            reasoning_details: None,
            finish_reason: Some(FinishReason::Stop), /* Should be from the last message with a
                                                      * finish reason */
            phase: None,
        };

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_into_full_with_finish_reason_tool_calls() {
        use crate::FinishReason;

        // Fixture: Create a stream that ends with a tool call finish reason
        let messages = vec![Ok(ChatCompletionMessage::default()
            .content(Content::part("I'll call a tool"))
            .finish_reason_opt(Some(FinishReason::ToolCalls)))];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: Should have the tool_calls finish reason
        let expected = ChatCompletionMessageFull {
            content: "I'll call a tool".to_string(),
            tool_calls: vec![],
            thought_signature: None,
            usage: Usage::default(),
            reasoning: None,
            reasoning_details: None,
            finish_reason: Some(FinishReason::ToolCalls),
            phase: None,
        };

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_into_full_with_no_finish_reason() {
        // Fixture: Create a stream with no finish reasons
        let messages = vec![
            Ok(ChatCompletionMessage::default().content(Content::part("Hello"))),
            Ok(ChatCompletionMessage::default().content(Content::part(" world"))),
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: finish_reason should be None
        let expected = ChatCompletionMessageFull {
            content: "Hello world".to_string(),
            tool_calls: vec![],
            thought_signature: None,
            usage: Usage::default(),
            reasoning: None,
            reasoning_details: None,
            finish_reason: None,
            phase: None,
        };

        assert_eq!(actual, expected);
    }
    #[tokio::test]
    async fn test_into_full_stream_continues_after_xml_interruption_for_usage_only() {
        let xml_content = r#"<forge_tool_call>
{"name": "test_tool", "arguments": {"arg": "value"}}
</forge_tool_call>"#;

        let messages = vec![
            Ok(ChatCompletionMessage::default().content(Content::part(xml_content))),
            // After interruption - content should be ignored but usage should be captured
            Ok(ChatCompletionMessage::default()
                .content(Content::part("This content should be ignored"))),
            Ok(ChatCompletionMessage::default()
                .content(Content::part("This too should be ignored"))),
            Ok(ChatCompletionMessage::default().usage(Usage {
                prompt_tokens: TokenCount::Actual(5),
                completion_tokens: TokenCount::Actual(20),
                total_tokens: TokenCount::Actual(25),
                cached_tokens: TokenCount::Actual(0),
                cost: None,
            })),
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message with XML interruption enabled
        let actual = result_stream.into_full(true).await.unwrap();

        // Expected: Should have XML tool call, content only from before interruption,
        // but final usage
        assert_eq!(actual.content, xml_content);
        assert_eq!(actual.tool_calls.len(), 1);
        assert_eq!(actual.tool_calls[0].name.as_str(), "test_tool");
        assert_eq!(actual.usage.total_tokens, TokenCount::Actual(25));
        assert_eq!(actual.usage.completion_tokens, TokenCount::Actual(20));
    }

    #[tokio::test]
    async fn test_into_full_empty_completion_creates_unparsed_tool_calls() {
        use crate::Error;

        // Fixture: Create a stream with empty content, no tool calls, and no finish
        // reason
        let messages = vec![
            Ok(ChatCompletionMessage::default()), // Completely empty message
            Ok(ChatCompletionMessage::default().content(Content::part(""))), // Empty content
            Ok(ChatCompletionMessage::default().content(Content::part("   "))), // Whitespace only
        ];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await;

        // Expected: Should return a retryable error for empty completion
        assert!(actual.is_err());
        let error = actual.unwrap_err();
        let domain_error = error.downcast_ref::<Error>();
        assert!(domain_error.is_some());
        assert!(matches!(domain_error.unwrap(), Error::Retryable(_)));
    }

    #[tokio::test]
    async fn test_into_full_empty_completion_with_finish_reason_should_not_error() {
        use crate::FinishReason;

        // Fixture: Create a stream with empty content but with finish reason
        let messages = vec![Ok(ChatCompletionMessage::default()
            .content(Content::part(""))
            .finish_reason_opt(Some(FinishReason::Stop)))];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: Should succeed because finish reason is present
        let expected = ChatCompletionMessageFull {
            content: "".to_string(),
            tool_calls: vec![],
            thought_signature: None,
            usage: Usage::default(),
            reasoning: None,
            reasoning_details: None,
            finish_reason: Some(FinishReason::Stop),
            phase: None,
        };

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_into_full_empty_completion_with_tool_calls_should_not_error() {
        // Fixture: Create a stream with empty content but with tool calls
        let tool_call = ToolCallFull {
            name: ToolName::new("test_tool"),
            call_id: Some(ToolCallId::new("call_123")),
            arguments: serde_json::json!("test_arg").into(),
            thought_signature: None,
        };

        let messages = vec![Ok(ChatCompletionMessage::default()
            .content(Content::part(""))
            .add_tool_call(ToolCall::Full(tool_call.clone())))];

        let result_stream: BoxStream<ChatCompletionMessage, anyhow::Error> =
            Box::pin(tokio_stream::iter(messages));

        // Actual: Convert stream to full message
        let actual = result_stream.into_full(false).await.unwrap();

        // Expected: Should succeed because tool calls are present
        let expected = ChatCompletionMessageFull {
            content: "".to_string(),
            tool_calls: vec![tool_call],
            thought_signature: None,
            usage: Usage::default(),
            reasoning: None,
            reasoning_details: None,
            finish_reason: None,
            phase: None,
        };

        assert_eq!(actual, expected);
    }
}
