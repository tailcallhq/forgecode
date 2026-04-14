use std::collections::{HashMap, HashSet};

use async_openai::types::responses as oai;
use forge_app::domain::{
    ChatCompletionMessage, Content, FinishReason, MessagePhase, TokenCount, ToolCall,
    ToolCallArguments, ToolCallFull, ToolCallId, ToolCallPart, ToolName, Usage,
};
use forge_domain::{BoxStream, ResultStream};
use futures::StreamExt;
use serde::{Deserialize, Deserializer};

use crate::provider::IntoDomain;

/// Wrapper enum for SSE events from the OpenAI Responses API.
///
/// Some OpenAI-compatible providers (including the Codex backend) send
/// `keepalive` heartbeat events in the stream. These events are not part of
/// `async_openai`'s `ResponseStreamEvent` enum, so we model them here to avoid
/// failing the entire stream.
///
/// Cost-bearing `ping` events from proxy servers (e.g. opencode.ai) are
/// captured and forwarded as usage data. Other unknown events are silently
/// ignored, matching the approach used by the Google and Anthropic providers.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(super) enum ResponsesStreamEvent {
    /// Heartbeat event containing only a sequence number.
    #[serde(rename = "keepalive")]
    Keepalive {
        #[allow(dead_code)]
        sequence_number: u64,
    },

    /// Cost-bearing heartbeat event sent by some proxies (e.g. opencode.ai).
    ///
    /// Example payload: `{"type":"ping","cost":"0.00675010"}`
    #[serde(rename = "ping")]
    Ping {
        #[serde(deserialize_with = "deserialize_string_or_f64")]
        cost: f64,
    },

    /// Any standard OpenAI Responses API streaming event.
    #[serde(untagged)]
    Response(Box<oai::ResponseStreamEvent>),

    /// Catch-all for any other unrecognised events. Silently ignored at the
    /// stream level.
    #[serde(untagged)]
    Unknown(#[allow(dead_code)] serde_json::Value),
}

/// Deserializes a value that may be either a JSON number or a numeric string
/// into an `f64`.
fn deserialize_string_or_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(n) => n
            .as_f64()
            .ok_or_else(|| serde::de::Error::custom("cost number is not representable as f64")),
        serde_json::Value::String(s) => s
            .parse::<f64>()
            .map_err(|e| serde::de::Error::custom(format!("invalid cost value: {e}"))),
        other => Err(serde::de::Error::custom(format!(
            "invalid cost type: expected number or string, got {other}"
        ))),
    }
}

/// Items that flow through the stream pipeline before final conversion to
/// `ChatCompletionMessage`.
///
/// Most events are standard OpenAI Responses API events that go through the
/// stateful `scan` conversion. Pre-resolved messages (e.g. from proxy `ping`
/// events carrying cost) bypass the scan and are passed through directly.
pub(super) enum StreamItem {
    /// A standard OpenAI Responses API streaming event.
    Event(Box<oai::ResponseStreamEvent>),
    /// A pre-resolved message (e.g. cost from a proxy ping event).
    Message(Box<ChatCompletionMessage>),
}

/// Converts OpenAI Responses API usage into the domain Usage type.
/// Usage is sent once in the `response.completed` event (not split across
/// events).
/// ref: https://developers.openai.com/api/reference/resources/responses#(resource)%20responses%20%3E%20(model)%20response_usage%20%3E%20(schema)
impl IntoDomain for oai::ResponseUsage {
    type Domain = Usage;

    fn into_domain(self) -> Self::Domain {
        Usage {
            prompt_tokens: TokenCount::Actual(self.input_tokens as usize),
            completion_tokens: TokenCount::Actual(self.output_tokens as usize),
            total_tokens: TokenCount::Actual(self.total_tokens as usize),
            cached_tokens: TokenCount::Actual(self.input_tokens_details.cached_tokens as usize),
            cost: None,
        }
    }
}

impl IntoDomain for oai::MessagePhase {
    type Domain = MessagePhase;

    fn into_domain(self) -> Self::Domain {
        match self {
            oai::MessagePhase::Commentary => MessagePhase::Commentary,
            oai::MessagePhase::FinalAnswer => MessagePhase::FinalAnswer,
        }
    }
}

impl IntoDomain for oai::Response {
    type Domain = ChatCompletionMessage;

    fn into_domain(self) -> Self::Domain {
        let mut message = ChatCompletionMessage::default();

        if let Some(text) = self.output_text() {
            message = message.content_full(text);
        }

        let mut saw_tool_call = false;
        for item in &self.output {
            match item {
                oai::OutputItem::Message(output_msg) => {
                    // Preserve phase from the assistant output message
                    if let Some(phase) = output_msg.phase {
                        message.phase = Some(phase.into_domain());
                    }
                }
                oai::OutputItem::FunctionCall(call) => {
                    saw_tool_call = true;
                    message = message.add_tool_call(ToolCall::Full(ToolCallFull {
                        call_id: Some(ToolCallId::new(call.call_id.clone())),
                        name: ToolName::new(call.name.clone()),
                        arguments: ToolCallArguments::from_json(&call.arguments),
                        thought_signature: None,
                    }));
                }
                oai::OutputItem::Reasoning(reasoning) => {
                    let mut all_reasoning_text = String::new();

                    if let Some(encrypted_content) = &reasoning.encrypted_content {
                        message =
                            message.add_reasoning_detail(forge_domain::Reasoning::Full(vec![
                                forge_domain::ReasoningFull {
                                    data: Some(encrypted_content.clone()),
                                    id: Some(reasoning.id.clone()),
                                    type_of: Some("reasoning.encrypted".to_string()),
                                    ..Default::default()
                                },
                            ]));
                    }

                    // Process reasoning text content
                    if let Some(content) = &reasoning.content {
                        let reasoning_text =
                            content.iter().map(|c| c.text.as_str()).collect::<String>();
                        if !reasoning_text.is_empty() {
                            all_reasoning_text.push_str(&reasoning_text);
                            message =
                                message.add_reasoning_detail(forge_domain::Reasoning::Full(vec![
                                    forge_domain::ReasoningFull {
                                        text: Some(reasoning_text),
                                        type_of: Some("reasoning.text".to_string()),
                                        id: Some(reasoning.id.clone()),
                                        ..Default::default()
                                    },
                                ]));
                        }
                    }

                    // Process reasoning summary - include the reasoning id so that
                    // summary parts can be grouped with their encrypted counterpart
                    // when replayed back to the API.
                    if !reasoning.summary.is_empty() {
                        let mut summary_texts = Vec::new();
                        for summary_part in &reasoning.summary {
                            match summary_part {
                                oai::SummaryPart::SummaryText(summary) => {
                                    summary_texts.push(summary.text.clone());
                                }
                            }
                        }
                        let summary_text = summary_texts.join("");
                        if !summary_text.is_empty() {
                            all_reasoning_text.push_str(&summary_text);
                            message =
                                message.add_reasoning_detail(forge_domain::Reasoning::Full(vec![
                                    forge_domain::ReasoningFull {
                                        text: Some(summary_text),
                                        type_of: Some("reasoning.summary".to_string()),
                                        id: Some(reasoning.id.clone()),
                                        ..Default::default()
                                    },
                                ]));
                        }
                    }

                    // Set the combined reasoning text in the reasoning field
                    if !all_reasoning_text.is_empty() {
                        message = message.reasoning(Content::full(all_reasoning_text));
                    }
                }
                _ => {}
            }
        }

        if let Some(usage) = self.usage {
            message = message.usage(usage.into_domain());
        }

        message = message.finish_reason_opt(Some(if saw_tool_call {
            FinishReason::ToolCalls
        } else {
            FinishReason::Stop
        }));

        message
    }
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, derive_more::From)]
struct ToolCallIndex(u32);

#[derive(Default)]
struct CodexStreamState {
    output_index_to_tool_call: HashMap<ToolCallIndex, (ToolCallId, ToolName)>,
    /// Tracks output indices that have received at least one arguments delta.
    /// When arguments are streamed via deltas, the `done` event should be
    /// skipped to avoid duplication. When no deltas are received (e.g. the
    /// Spark model sends arguments only in the `done` event), we must emit
    /// them from the `done` handler.
    received_toolcall_deltas: HashSet<ToolCallIndex>,
}

/// Retains only reasoning details that carry `encrypted_content` data.
///
/// During streaming, reasoning text and summary parts are already emitted
/// via delta events. However, `encrypted_content` (type `reasoning.encrypted`)
/// is only available in the final `ResponseCompleted`/`ResponseIncomplete`
/// event. This function filters out text/summary reasoning details (which would
/// be duplicated) and keeps only the encrypted content entries that are
/// required for stateless multi-turn reasoning replay.
fn retain_encrypted_reasoning_details(
    details: Option<Vec<forge_domain::Reasoning>>,
) -> Option<Vec<forge_domain::Reasoning>> {
    let details = details?;
    let encrypted: Vec<forge_domain::Reasoning> = details
        .into_iter()
        .filter(|r| {
            r.as_full().is_some_and(|fulls| {
                fulls
                    .iter()
                    .any(|f| f.type_of.as_deref() == Some("reasoning.encrypted"))
            })
        })
        .collect();
    if encrypted.is_empty() {
        None
    } else {
        Some(encrypted)
    }
}

impl IntoDomain for BoxStream<StreamItem, anyhow::Error> {
    type Domain = ResultStream<ChatCompletionMessage, anyhow::Error>;

    fn into_domain(self) -> Self::Domain {
        Ok(Box::pin(
            self.scan(CodexStreamState::default(), move |state, item| {
                futures::future::ready({
                    let item = match item {
                        Ok(StreamItem::Message(msg)) => Some(Ok(*msg)),
                        Ok(StreamItem::Event(event)) => match *event {
                            oai::ResponseStreamEvent::ResponseOutputTextDelta(delta) => Some(Ok(
                                ChatCompletionMessage::assistant(Content::part(delta.delta)),
                            )),
                            oai::ResponseStreamEvent::ResponseReasoningTextDelta(delta) => {
                                Some(Ok(ChatCompletionMessage::default()
                                    .reasoning(Content::part(delta.delta.clone()))
                                    .add_reasoning_detail(forge_domain::Reasoning::Part(vec![
                                        forge_domain::ReasoningPart {
                                            text: Some(delta.delta),
                                            id: Some(delta.item_id),
                                            type_of: Some("reasoning.text".to_string()),
                                            ..Default::default()
                                        },
                                    ]))))
                            }
                            oai::ResponseStreamEvent::ResponseReasoningSummaryTextDelta(delta) => {
                                Some(Ok(ChatCompletionMessage::default()
                                    .reasoning(Content::part(delta.delta.clone()))
                                    .add_reasoning_detail(forge_domain::Reasoning::Part(vec![
                                        forge_domain::ReasoningPart {
                                            text: Some(delta.delta),
                                            id: Some(delta.item_id),
                                            type_of: Some("reasoning.summary".to_string()),
                                            ..Default::default()
                                        },
                                    ]))))
                            }
                            oai::ResponseStreamEvent::ResponseOutputItemAdded(added) => {
                                match &added.item {
                                    oai::OutputItem::FunctionCall(call) => {
                                        let tool_call_id = ToolCallId::new(call.call_id.clone());
                                        let tool_name = ToolName::new(call.name.clone());

                                        state.output_index_to_tool_call.insert(
                                            added.output_index.into(),
                                            (tool_call_id.clone(), tool_name.clone()),
                                        );

                                        // Only emit if we have non-empty initial arguments.
                                        // Otherwise, wait for deltas or done event.
                                        if !call.arguments.is_empty() {
                                            Some(Ok(ChatCompletionMessage::default()
                                                .add_tool_call(ToolCall::Part(ToolCallPart {
                                                    call_id: Some(tool_call_id),
                                                    name: Some(tool_name),
                                                    arguments_part: call.arguments.clone(),
                                                    thought_signature: None,
                                                }))))
                                        } else {
                                            None
                                        }
                                    }
                                    oai::OutputItem::Reasoning(_reasoning) => {
                                        // Reasoning items don't emit content in real-time, only at
                                        // completion
                                        None
                                    }
                                    _ => None,
                                }
                            }
                            oai::ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(delta) => {
                                state
                                    .received_toolcall_deltas
                                    .insert(delta.output_index.into());
                                let (call_id, name) = state
                                    .output_index_to_tool_call
                                    .get(&(delta.output_index.into()))
                                    .cloned()
                                    .unwrap_or_else(|| {
                                        (
                                            ToolCallId::new(format!(
                                                "output_{}",
                                                delta.output_index
                                            )),
                                            ToolName::new(""),
                                        )
                                    });

                                let name = (!name.as_str().is_empty()).then_some(name);

                                Some(Ok(ChatCompletionMessage::default().add_tool_call(
                                    ToolCall::Part(ToolCallPart {
                                        call_id: Some(call_id),
                                        name,
                                        arguments_part: delta.delta,
                                        thought_signature: None,
                                    }),
                                )))
                            }
                            oai::ResponseStreamEvent::ResponseFunctionCallArgumentsDone(done) => {
                                // If deltas were already streamed for this output index,
                                // the arguments have already been emitted incrementally.
                                if state
                                    .received_toolcall_deltas
                                    .contains(&(done.output_index.into()))
                                {
                                    None
                                } else {
                                    // No deltas were received (e.g. the Spark model sends
                                    // the complete arguments only in the `done` event).
                                    // Emit the full tool call now.
                                    let (call_id, name) = state
                                        .output_index_to_tool_call
                                        .get(&(done.output_index.into()))
                                        .cloned()
                                        .unwrap_or_else(|| {
                                            (
                                                ToolCallId::new(format!(
                                                    "output_{}",
                                                    done.output_index
                                                )),
                                                ToolName::new(
                                                    done.name.clone().unwrap_or_default(),
                                                ),
                                            )
                                        });

                                    let name = (!name.as_str().is_empty()).then_some(name);

                                    Some(Ok(ChatCompletionMessage::default().add_tool_call(
                                        ToolCall::Part(ToolCallPart {
                                            call_id: Some(call_id),
                                            name,
                                            arguments_part: done.arguments,
                                            thought_signature: None,
                                        }),
                                    )))
                                }
                            }
                            oai::ResponseStreamEvent::ResponseCompleted(done) => {
                                // Text content, reasoning, and tool calls were already streamed via
                                // delta events Only emit metadata
                                // (usage, finish_reason)
                                let mut message: ChatCompletionMessage =
                                    done.response.into_domain();
                                message.content = None; // Clear content to avoid duplication
                                message.reasoning = None; // Clear reasoning to avoid duplication
                                // Keep only encrypted-content reasoning details — text and
                                // summary were already streamed via deltas but
                                // encrypted_content is never streamed and must be preserved
                                // for multi-turn reasoning replay.
                                message.reasoning_details =
                                    retain_encrypted_reasoning_details(message.reasoning_details);
                                message.tool_calls.clear(); // Clear tool calls to avoid duplication
                                Some(Ok(message))
                            }
                            oai::ResponseStreamEvent::ResponseIncomplete(done) => {
                                // Text content, reasoning, and tool calls were already streamed via
                                // delta events
                                let mut message: ChatCompletionMessage =
                                    done.response.into_domain();
                                message.content = None; // Clear content to avoid duplication
                                message.reasoning = None; // Clear reasoning to avoid duplication
                                // Keep only encrypted-content reasoning details (see above).
                                message.reasoning_details =
                                    retain_encrypted_reasoning_details(message.reasoning_details);
                                message.tool_calls.clear(); // Clear tool calls to avoid duplication
                                message = message.finish_reason_opt(Some(FinishReason::Length));
                                Some(Ok(message))
                            }
                            oai::ResponseStreamEvent::ResponseFailed(failed) => {
                                Some(Err(anyhow::anyhow!(
                                    "Upstream response failed: {:?}",
                                    failed.response.error
                                )))
                            }
                            oai::ResponseStreamEvent::ResponseError(err) => {
                                Some(Err(anyhow::anyhow!("Upstream error: {}", err.message)))
                            }
                            _ => None,
                        },
                        Err(err) => Some(Err(err)),
                    };

                    Some(item)
                })
            })
            .filter_map(|item| async move { item }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use async_openai::types::responses as oai;

    // Type alias for ResponseStream in tests since it's not provided by
    // response-types
    type ResponseStream =
        std::pin::Pin<Box<dyn futures::Stream<Item = anyhow::Result<StreamItem>> + Send>>;
    use forge_app::domain::{Content, FinishReason, Reasoning, ReasoningFull, TokenCount, Usage};
    use forge_domain::{ChatCompletionMessage as Message, ToolCallId, ToolName};
    use tokio_stream::StreamExt;

    use super::*;

    // ============== Common Fixtures ==============

    /// Wraps an `oai::ResponseStreamEvent` into a `StreamItem::Event` result
    /// for use in test streams.
    fn event(e: oai::ResponseStreamEvent) -> anyhow::Result<StreamItem> {
        Ok(StreamItem::Event(Box::new(e)))
    }

    fn fixture_response_usage() -> oai::ResponseUsage {
        oai::ResponseUsage {
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: 150,
            input_tokens_details: oai::InputTokenDetails { cached_tokens: 20 },
            output_tokens_details: oai::OutputTokenDetails { reasoning_tokens: 0 },
        }
    }

    fn fixture_response_base(status: &str) -> oai::Response {
        serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "created_at": 0,
            "model": "codex-mini-latest",
            "object": "response",
            "status": status,
            "output": []
        }))
        .unwrap()
    }

    fn fixture_response_with_text(text: &str) -> oai::Response {
        serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "created_at": 0,
            "model": "codex-mini-latest",
            "object": "response",
            "status": "completed",
            "output": [
                {
                    "id": "msg_1",
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {
                            "type": "output_text",
                            "text": text,
                            "annotations": []
                        }
                    ],
                    "status": "completed"
                }
            ]
        }))
        .unwrap()
    }

    fn fixture_response_with_function_call(call_id: &str, name: &str, args: &str) -> oai::Response {
        serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "created_at": 0,
            "model": "codex-mini-latest",
            "object": "response",
            "status": "completed",
            "output": [
                {
                    "type": "function_call",
                    "call_id": call_id,
                    "name": name,
                    "arguments": args
                }
            ]
        }))
        .unwrap()
    }

    fn fixture_response_with_reasoning_text(text: &str) -> oai::Response {
        serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "created_at": 0,
            "model": "codex-mini-latest",
            "object": "response",
            "status": "completed",
            "output": [
                {
                    "id": "reasoning_1",
                    "type": "reasoning",
                    "content": [
                        {
                            "type": "reasoning_text",
                            "text": text
                        }
                    ],
                    "summary": [],
                    "annotations": []
                }
            ]
        }))
        .unwrap()
    }

    fn fixture_response_with_reasoning_summary(summary: &str) -> oai::Response {
        serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "created_at": 0,
            "model": "codex-mini-latest",
            "object": "response",
            "status": "completed",
            "output": [
                {
                    "id": "reasoning_1",
                    "type": "reasoning",
                    "summary": [
                        {
                            "type": "summary_text",
                            "text": summary
                        }
                    ],
                    "annotations": []
                }
            ]
        }))
        .unwrap()
    }

    fn fixture_response_with_reasoning_encrypted(encrypted: &str, id: &str) -> oai::Response {
        serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "created_at": 0,
            "model": "codex-mini-latest",
            "object": "response",
            "status": "completed",
            "output": [
                {
                    "id": id,
                    "type": "reasoning",
                    "summary": [],
                    "encrypted_content": encrypted,
                    "annotations": []
                }
            ]
        }))
        .unwrap()
    }

    fn fixture_response_with_reasoning_both(reasoning_text: &str, summary: &str) -> oai::Response {
        serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "created_at": 0,
            "model": "codex-mini-latest",
            "object": "response",
            "status": "completed",
            "output": [
                {
                    "id": "reasoning_1",
                    "type": "reasoning",
                    "content": [
                        {
                            "type": "reasoning_text",
                            "text": reasoning_text
                        }
                    ],
                    "summary": [
                        {
                            "type": "summary_text",
                            "text": summary
                        }
                    ],
                    "annotations": []
                }
            ]
        }))
        .unwrap()
    }

    fn fixture_response_with_usage(text: &str) -> oai::Response {
        serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "created_at": 0,
            "model": "codex-mini-latest",
            "object": "response",
            "status": "completed",
            "output": [
                {
                    "id": "msg_1",
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {
                            "type": "output_text",
                            "text": text,
                            "annotations": []
                        }
                    ],
                    "status": "completed"
                }
            ],
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "total_tokens": 150,
                "input_tokens_details": {
                    "cached_tokens": 20
                },
                "output_tokens_details": {
                    "reasoning_tokens": 0
                }
            }
        }))
        .unwrap()
    }

    fn fixture_response_failed() -> oai::Response {
        serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "created_at": 0,
            "model": "codex-mini-latest",
            "object": "response",
            "status": "failed",
            "output": [],
            "error": {
                "code": "rate_limit",
                "message": "Rate limit exceeded",
                "type": "invalid_request_error"
            }
        }))
        .unwrap()
    }

    fn fixture_response_incomplete(text: &str) -> oai::Response {
        serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "created_at": 0,
            "model": "codex-mini-latest",
            "object": "response",
            "status": "incomplete",
            "output": [
                {
                    "id": "msg_1",
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {
                            "type": "output_text",
                            "text": text,
                            "annotations": []
                        }
                    ],
                    "status": "incomplete"
                }
            ]
        }))
        .unwrap()
    }

    fn fixture_delta_text(delta: &str) -> oai::ResponseTextDeltaEvent {
        oai::ResponseTextDeltaEvent {
            sequence_number: 1,
            item_id: "item_1".to_string(),
            output_index: 0,
            content_index: 0,
            delta: delta.to_string(),
            logprobs: None,
        }
    }

    fn fixture_delta_reasoning_text(delta: &str) -> oai::ResponseReasoningTextDeltaEvent {
        oai::ResponseReasoningTextDeltaEvent {
            sequence_number: 1,
            item_id: "item_1".to_string(),
            output_index: 0,
            content_index: 0,
            delta: delta.to_string(),
        }
    }

    fn fixture_delta_reasoning_summary(delta: &str) -> oai::ResponseReasoningSummaryTextDeltaEvent {
        oai::ResponseReasoningSummaryTextDeltaEvent {
            sequence_number: 1,
            item_id: "item_1".to_string(),
            output_index: 0,
            summary_index: 0,
            delta: delta.to_string(),
        }
    }

    fn fixture_function_call_added(
        call_id: &str,
        name: &str,
        arguments: &str,
    ) -> oai::ResponseOutputItemAddedEvent {
        oai::ResponseOutputItemAddedEvent {
            sequence_number: 1,
            output_index: 0,
            item: serde_json::from_value(serde_json::json!({
                "type": "function_call",
                "call_id": call_id,
                "name": name,
                "arguments": arguments
            }))
            .unwrap(),
        }
    }

    fn fixture_reasoning_added() -> oai::ResponseOutputItemAddedEvent {
        oai::ResponseOutputItemAddedEvent {
            sequence_number: 1,
            output_index: 0,
            item: serde_json::from_value(serde_json::json!({
                "id": "reasoning_1",
                "type": "reasoning",
                "summary": [],
                "annotations": []
            }))
            .unwrap(),
        }
    }

    fn fixture_function_call_arguments_delta(
        output_index: u32,
        delta: &str,
    ) -> oai::ResponseFunctionCallArgumentsDeltaEvent {
        oai::ResponseFunctionCallArgumentsDeltaEvent {
            sequence_number: 2,
            item_id: "item_1".to_string(),
            output_index,
            delta: delta.to_string(),
        }
    }

    fn fixture_response_error_event() -> oai::ResponseErrorEvent {
        oai::ResponseErrorEvent {
            sequence_number: 1,
            code: Some("connection_error".to_string()),
            message: "Connection error".to_string(),
            param: None,
        }
    }

    fn fixture_expected_usage() -> Usage {
        Usage {
            prompt_tokens: TokenCount::Actual(100),
            completion_tokens: TokenCount::Actual(50),
            total_tokens: TokenCount::Actual(150),
            cached_tokens: TokenCount::Actual(20),
            cost: None,
        }
    }

    // ============== ResponseUsage Tests ==============

    #[test]
    fn test_response_usage_into_domain() {
        let fixture = fixture_response_usage();
        let actual = fixture.into_domain();
        let expected = fixture_expected_usage();

        assert_eq!(actual, expected);
    }

    // ============== Response Tests ==============

    #[test]
    fn test_response_into_domain_with_text_only() {
        let fixture = fixture_response_with_text("Hello world");
        let actual = fixture.into_domain();

        assert_eq!(actual.content, Some(Content::full("Hello world")));
        assert_eq!(actual.finish_reason, Some(FinishReason::Stop));
        assert!(actual.tool_calls.is_empty());
    }

    #[test]
    fn test_response_into_domain_preserves_commentary_phase() {
        let fixture: oai::Response = serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "created_at": 0,
            "model": "codex-mini-latest",
            "object": "response",
            "status": "completed",
            "output": [
                {
                    "id": "msg_1",
                    "type": "message",
                    "role": "assistant",
                    "phase": "commentary",
                    "content": [
                        {
                            "type": "output_text",
                            "text": "Thinking...",
                            "annotations": []
                        }
                    ],
                    "status": "completed"
                }
            ]
        }))
        .unwrap();
        let actual = fixture.into_domain();

        assert_eq!(
            actual.phase,
            Some(forge_app::domain::MessagePhase::Commentary)
        );
        assert_eq!(actual.content, Some(Content::full("Thinking...")));
    }

    #[test]
    fn test_response_into_domain_preserves_final_answer_phase() {
        let fixture: oai::Response = serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "created_at": 0,
            "model": "codex-mini-latest",
            "object": "response",
            "status": "completed",
            "output": [
                {
                    "id": "msg_1",
                    "type": "message",
                    "role": "assistant",
                    "phase": "final_answer",
                    "content": [
                        {
                            "type": "output_text",
                            "text": "The answer is 42.",
                            "annotations": []
                        }
                    ],
                    "status": "completed"
                }
            ]
        }))
        .unwrap();
        let actual = fixture.into_domain();

        assert_eq!(
            actual.phase,
            Some(forge_app::domain::MessagePhase::FinalAnswer)
        );
        assert_eq!(actual.content, Some(Content::full("The answer is 42.")));
    }

    #[test]
    fn test_response_into_domain_no_phase_when_absent() {
        let fixture = fixture_response_with_text("Hello");
        let actual = fixture.into_domain();

        assert_eq!(actual.phase, None);
    }

    #[test]
    fn test_response_into_domain_with_function_call() {
        let fixture =
            fixture_response_with_function_call("call_123", "shell", r#"{"cmd":"echo hi"}"#);
        let actual = fixture.into_domain();

        assert_eq!(actual.tool_calls.len(), 1);
        assert_eq!(actual.finish_reason, Some(FinishReason::ToolCalls));
        assert!(actual.content.is_none());
    }

    #[test]
    fn test_response_into_domain_with_reasoning_text() {
        let fixture = fixture_response_with_reasoning_text("This is my reasoning");
        let actual = fixture.into_domain();

        assert_eq!(
            actual.reasoning,
            Some(Content::full("This is my reasoning"))
        );
        assert_eq!(
            actual.reasoning_details,
            Some(vec![Reasoning::Full(vec![ReasoningFull {
                text: Some("This is my reasoning".to_string()),
                type_of: Some("reasoning.text".to_string()),
                id: Some("reasoning_1".to_string()),
                ..Default::default()
            }])])
        );
        assert_eq!(actual.finish_reason, Some(FinishReason::Stop));
    }

    #[test]
    fn test_response_into_domain_with_reasoning_summary() {
        let fixture = fixture_response_with_reasoning_summary("Summary of reasoning");
        let actual = fixture.into_domain();

        assert_eq!(
            actual.reasoning,
            Some(Content::full("Summary of reasoning"))
        );
        assert_eq!(
            actual.reasoning_details,
            Some(vec![Reasoning::Full(vec![ReasoningFull {
                text: Some("Summary of reasoning".to_string()),
                type_of: Some("reasoning.summary".to_string()),
                id: Some("reasoning_1".to_string()),
                ..Default::default()
            }])])
        );
        assert_eq!(actual.finish_reason, Some(FinishReason::Stop));
    }

    #[test]
    fn test_response_into_domain_with_reasoning_encrypted_content() {
        let fixture = fixture_response_with_reasoning_encrypted("enc_payload_abc", "reasoning_1");
        let actual = fixture.into_domain();

        assert_eq!(actual.reasoning, None);
        assert_eq!(
            actual.reasoning_details,
            Some(vec![Reasoning::Full(vec![ReasoningFull {
                data: Some("enc_payload_abc".to_string()),
                id: Some("reasoning_1".to_string()),
                type_of: Some("reasoning.encrypted".to_string()),
                ..Default::default()
            }])])
        );
        assert_eq!(actual.finish_reason, Some(FinishReason::Stop));
    }

    #[test]
    fn test_response_into_domain_with_reasoning_text_and_summary() {
        let fixture = fixture_response_with_reasoning_both("Reasoning text", "Summary");
        let actual = fixture.into_domain();

        assert_eq!(
            actual.reasoning,
            Some(Content::full("Reasoning textSummary"))
        );
        assert_eq!(
            actual.reasoning_details,
            Some(vec![
                Reasoning::Full(vec![ReasoningFull {
                    text: Some("Reasoning text".to_string()),
                    type_of: Some("reasoning.text".to_string()),
                    id: Some("reasoning_1".to_string()),
                    ..Default::default()
                }]),
                Reasoning::Full(vec![ReasoningFull {
                    text: Some("Summary".to_string()),
                    type_of: Some("reasoning.summary".to_string()),
                    id: Some("reasoning_1".to_string()),
                    ..Default::default()
                }]),
            ])
        );
    }

    #[test]
    fn test_response_into_domain_with_usage() {
        let fixture = fixture_response_with_usage("Hello");
        let actual = fixture.into_domain();

        assert_eq!(actual.usage, Some(fixture_expected_usage()));
    }

    // ============== ResponseStream Tests ==============

    #[tokio::test]
    async fn test_stream_with_output_text_delta() -> anyhow::Result<()> {
        let delta = fixture_delta_text("hello");

        let stream: ResponseStream = Box::pin(tokio_stream::iter([event(
            oai::ResponseStreamEvent::ResponseOutputTextDelta(delta),
        )]));

        let mut stream_domain = stream.into_domain()?;
        let actual: Message = stream_domain.next().await.unwrap()?;

        assert_eq!(actual.content, Some(Content::part("hello")));

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_with_reasoning_text_delta() -> anyhow::Result<()> {
        let delta = fixture_delta_reasoning_text("thinking...");

        let stream: ResponseStream = Box::pin(tokio_stream::iter([event(
            oai::ResponseStreamEvent::ResponseReasoningTextDelta(delta),
        )]));

        let mut stream_domain = stream.into_domain()?;
        let actual: Message = stream_domain.next().await.unwrap()?;

        assert_eq!(actual.reasoning, Some(Content::part("thinking...")));
        assert_eq!(
            actual.reasoning_details,
            Some(vec![Reasoning::Part(vec![forge_domain::ReasoningPart {
                text: Some("thinking...".to_string()),
                id: Some("item_1".to_string()),
                type_of: Some("reasoning.text".to_string()),
                ..Default::default()
            }])])
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_with_reasoning_summary_text_delta() -> anyhow::Result<()> {
        let delta = fixture_delta_reasoning_summary("summary...");

        let stream: ResponseStream = Box::pin(tokio_stream::iter([event(
            oai::ResponseStreamEvent::ResponseReasoningSummaryTextDelta(delta),
        )]));

        let mut stream_domain = stream.into_domain()?;
        let actual: Message = stream_domain.next().await.unwrap()?;

        assert_eq!(actual.reasoning, Some(Content::part("summary...")));
        assert_eq!(
            actual.reasoning_details,
            Some(vec![Reasoning::Part(vec![forge_domain::ReasoningPart {
                text: Some("summary...".to_string()),
                id: Some("item_1".to_string()),
                type_of: Some("reasoning.summary".to_string()),
                ..Default::default()
            }])])
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_with_function_call_added_with_arguments() -> anyhow::Result<()> {
        let added = fixture_function_call_added("call_123", "shell", r#"{"cmd":"echo"}"#);

        let stream: ResponseStream = Box::pin(tokio_stream::iter([event(
            oai::ResponseStreamEvent::ResponseOutputItemAdded(added),
        )]));

        let mut stream_domain = stream.into_domain()?;
        let actual: Message = stream_domain.next().await.unwrap()?;

        assert_eq!(actual.tool_calls.len(), 1);
        let tool_call = actual.tool_calls.first().unwrap();
        let part = tool_call.as_partial().unwrap();
        assert_eq!(
            part.call_id.as_ref().map(|id: &ToolCallId| id.as_str()),
            Some("call_123")
        );
        assert_eq!(
            part.name.as_ref().map(|n: &ToolName| n.as_str()),
            Some("shell")
        );
        assert_eq!(part.arguments_part, r#"{"cmd":"echo"}"#);

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_with_function_call_added_without_arguments() -> anyhow::Result<()> {
        let added = fixture_function_call_added("call_123", "shell", "");

        let stream: ResponseStream = Box::pin(tokio_stream::iter([event(
            oai::ResponseStreamEvent::ResponseOutputItemAdded(added),
        )]));

        let mut stream_domain = stream.into_domain()?;
        let actual = stream_domain.next().await;

        // Should not emit when arguments are empty
        assert!(actual.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_with_reasoning_added() -> anyhow::Result<()> {
        let added = fixture_reasoning_added();

        let stream: ResponseStream = Box::pin(tokio_stream::iter([event(
            oai::ResponseStreamEvent::ResponseOutputItemAdded(added),
        )]));

        let mut stream_domain = stream.into_domain()?;
        let actual = stream_domain.next().await;

        // Reasoning items don't emit content in real-time
        assert!(actual.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_with_function_call_arguments_delta() -> anyhow::Result<()> {
        let added = fixture_function_call_added("call_123", "shell", "");
        let delta = fixture_function_call_arguments_delta(0, r#"{"cmd":"echo"}"#);

        let stream: ResponseStream = Box::pin(tokio_stream::iter([
            event(oai::ResponseStreamEvent::ResponseOutputItemAdded(added)),
            event(oai::ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(delta)),
        ]));

        let mut stream_domain = stream.into_domain()?;
        let actual: Message = stream_domain.next().await.unwrap()?;

        assert_eq!(actual.tool_calls.len(), 1);
        let tool_call = actual.tool_calls.first().unwrap();
        let part = tool_call.as_partial().unwrap();
        assert_eq!(
            part.call_id.as_ref().map(|id: &ToolCallId| id.as_str()),
            Some("call_123")
        );
        assert_eq!(
            part.name.as_ref().map(|n: &ToolName| n.as_str()),
            Some("shell")
        );
        assert_eq!(part.arguments_part, r#"{"cmd":"echo"}"#);

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_with_function_call_arguments_delta_unknown_index() -> anyhow::Result<()> {
        let delta = fixture_function_call_arguments_delta(999, r#"{"cmd":"echo"}"#);

        let stream: ResponseStream = Box::pin(tokio_stream::iter([event(
            oai::ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(delta),
        )]));

        let mut stream_domain = stream.into_domain()?;
        let actual: Message = stream_domain.next().await.unwrap()?;

        assert_eq!(actual.tool_calls.len(), 1);
        let tool_call = actual.tool_calls.first().unwrap();
        let part = tool_call.as_partial().unwrap();
        assert_eq!(
            part.call_id.as_ref().map(|id: &ToolCallId| id.as_str()),
            Some("output_999")
        );
        assert!(part.name.is_none());
        assert_eq!(part.arguments_part, r#"{"cmd":"echo"}"#);

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_with_function_call_arguments_done_no_deltas() -> anyhow::Result<()> {
        // When no deltas were received, the done event should emit the tool call
        let done = oai::ResponseFunctionCallArgumentsDoneEvent {
            sequence_number: 1,
            output_index: 0,
            item_id: "item_1".to_string(),
            name: Some("shell".to_string()),
            arguments: r#"{"cmd":"echo hi"}"#.to_string(),
        };

        let stream: ResponseStream = Box::pin(tokio_stream::iter([event(
            oai::ResponseStreamEvent::ResponseFunctionCallArgumentsDone(done),
        )]));

        let mut stream_domain = stream.into_domain()?;
        let actual: Message = stream_domain.next().await.unwrap()?;

        assert_eq!(actual.tool_calls.len(), 1);
        let tool_call = actual.tool_calls.first().unwrap();
        let part = tool_call.as_partial().unwrap();
        assert_eq!(
            part.call_id.as_ref().map(|id: &ToolCallId| id.as_str()),
            Some("output_0")
        );
        assert_eq!(
            part.name.as_ref().map(|n: &ToolName| n.as_str()),
            Some("shell")
        );
        assert_eq!(part.arguments_part, r#"{"cmd":"echo hi"}"#);

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_with_function_call_arguments_done_after_deltas() -> anyhow::Result<()> {
        // When deltas were already received, the done event should NOT emit
        let added = fixture_function_call_added("call_123", "shell", "");
        let delta = fixture_function_call_arguments_delta(0, r#"{"cmd":"echo"}"#);
        let done = oai::ResponseFunctionCallArgumentsDoneEvent {
            sequence_number: 3,
            output_index: 0,
            item_id: "item_1".to_string(),
            name: Some("shell".to_string()),
            arguments: r#"{"cmd":"echo"}"#.to_string(),
        };

        let stream: ResponseStream = Box::pin(tokio_stream::iter([
            event(oai::ResponseStreamEvent::ResponseOutputItemAdded(added)),
            event(oai::ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(delta)),
            event(oai::ResponseStreamEvent::ResponseFunctionCallArgumentsDone(
                done,
            )),
        ]));

        let mut stream_domain = stream.into_domain()?;
        let mut messages = vec![];
        while let Some(msg) = stream_domain.next().await {
            messages.push(msg);
        }

        // Should only get one message from the delta, not a duplicate from done
        assert_eq!(messages.len(), 1);
        let actual = messages.remove(0)?;
        assert_eq!(actual.tool_calls.len(), 1);
        let part = actual.tool_calls.first().unwrap().as_partial().unwrap();
        assert_eq!(part.arguments_part, r#"{"cmd":"echo"}"#);

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_with_response_completed() -> anyhow::Result<()> {
        let response = fixture_response_with_text("Final message");
        let completed = oai::ResponseCompletedEvent { sequence_number: 2, response };

        let stream: ResponseStream = Box::pin(tokio_stream::iter([event(
            oai::ResponseStreamEvent::ResponseCompleted(completed),
        )]));

        let mut stream_domain = stream.into_domain()?;
        let actual: Message = stream_domain.next().await.unwrap()?;

        // Content is cleared in completion events since it was already streamed
        assert_eq!(actual.content, None);
        assert_eq!(actual.finish_reason, Some(FinishReason::Stop));

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_with_response_incomplete() -> anyhow::Result<()> {
        let response = fixture_response_incomplete("Partial message");
        let incomplete = oai::ResponseIncompleteEvent { sequence_number: 2, response };

        let stream: ResponseStream = Box::pin(tokio_stream::iter([event(
            oai::ResponseStreamEvent::ResponseIncomplete(incomplete),
        )]));

        let mut stream_domain = stream.into_domain()?;
        let actual: Message = stream_domain.next().await.unwrap()?;

        // Content is cleared since it was already streamed
        assert_eq!(actual.content, None);
        assert_eq!(actual.finish_reason, Some(FinishReason::Length));

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_with_response_failed() -> anyhow::Result<()> {
        let response = fixture_response_failed();
        let failed = oai::ResponseFailedEvent { sequence_number: 2, response };

        let stream: ResponseStream = Box::pin(tokio_stream::iter([event(
            oai::ResponseStreamEvent::ResponseFailed(failed),
        )]));

        let mut stream_domain = stream.into_domain()?;
        let actual = stream_domain.next().await.unwrap();

        assert!(actual.is_err());
        assert!(
            actual
                .unwrap_err()
                .to_string()
                .contains("Upstream response failed")
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_with_response_error() -> anyhow::Result<()> {
        let error = fixture_response_error_event();

        let stream: ResponseStream = Box::pin(tokio_stream::iter([event(
            oai::ResponseStreamEvent::ResponseError(error),
        )]));

        let mut stream_domain = stream.into_domain()?;
        let actual = stream_domain.next().await.unwrap();

        assert!(actual.is_err());
        assert!(actual.unwrap_err().to_string().contains("Upstream error"));

        Ok(())
    }

    #[tokio::test]
    async fn test_into_chat_completion_message_codex_maps_text_and_finish() -> anyhow::Result<()> {
        let delta = fixture_delta_text("hello");
        let response = fixture_response_base("completed");
        let completed = oai::ResponseCompletedEvent { sequence_number: 2, response };

        let stream: ResponseStream = Box::pin(tokio_stream::iter([
            event(oai::ResponseStreamEvent::ResponseOutputTextDelta(delta)),
            event(oai::ResponseStreamEvent::ResponseCompleted(completed)),
        ]));

        let mut stream_domain = stream.into_domain()?;
        let mut actual = vec![];
        while let Some(msg) = stream_domain.next().await {
            actual.push(msg);
        }

        let first = actual.remove(0)?;
        assert_eq!(first.content, Some(Content::part("hello")));

        let second = actual.remove(0)?;
        assert_eq!(second.finish_reason, Some(FinishReason::Stop));

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_with_multiple_function_call_deltas() -> anyhow::Result<()> {
        let added = fixture_function_call_added("call_123", "shell", "");
        let delta1 = fixture_function_call_arguments_delta(0, r#"{"cmd":"echo"#);
        let delta2 = fixture_function_call_arguments_delta(0, r#" hi"}"#);

        let stream: ResponseStream = Box::pin(tokio_stream::iter([
            event(oai::ResponseStreamEvent::ResponseOutputItemAdded(added)),
            event(oai::ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(delta1)),
            event(oai::ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(delta2)),
        ]));

        let mut stream_domain = stream.into_domain()?;
        let mut messages: Vec<anyhow::Result<Message>> = vec![];

        while let Some(msg) = stream_domain.next().await {
            messages.push(msg);
        }

        assert_eq!(messages.len(), 2);

        // First delta
        let first = messages.remove(0).unwrap();
        assert_eq!(first.tool_calls.len(), 1);
        let part1 = first.tool_calls[0].as_partial().unwrap();
        assert_eq!(
            part1.call_id.as_ref().map(|id: &ToolCallId| id.as_str()),
            Some("call_123")
        );
        assert_eq!(
            part1.name.as_ref().map(|n: &ToolName| n.as_str()),
            Some("shell")
        );
        assert_eq!(part1.arguments_part, r#"{"cmd":"echo"#);

        // Second delta
        let second = messages.remove(0).unwrap();
        assert_eq!(second.tool_calls.len(), 1);
        let part2 = second.tool_calls[0].as_partial().unwrap();
        assert_eq!(
            part2.call_id.as_ref().map(|id: &ToolCallId| id.as_str()),
            Some("call_123")
        );
        assert_eq!(
            part2.name.as_ref().map(|n: &ToolName| n.as_str()),
            Some("shell")
        );
        assert_eq!(part2.arguments_part, r#" hi"}"#);

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_avoids_duplicate_content_in_completion() -> anyhow::Result<()> {
        // Simulate realistic streaming: deltas followed by completion event
        let delta1 = fixture_delta_text("<commit_message>");
        let delta2 = fixture_delta_text("fix: avoid duplication");
        let delta3 = fixture_delta_text("</commit_message>");

        // Completion event contains the full text that was already streamed
        let response =
            fixture_response_with_text("<commit_message>fix: avoid duplication</commit_message>");
        let completed = oai::ResponseCompletedEvent { sequence_number: 4, response };

        let stream: ResponseStream = Box::pin(tokio_stream::iter([
            event(oai::ResponseStreamEvent::ResponseOutputTextDelta(delta1)),
            event(oai::ResponseStreamEvent::ResponseOutputTextDelta(delta2)),
            event(oai::ResponseStreamEvent::ResponseOutputTextDelta(delta3)),
            event(oai::ResponseStreamEvent::ResponseCompleted(completed)),
        ]));

        let mut stream_domain = stream.into_domain()?;
        let mut messages: Vec<anyhow::Result<Message>> = vec![];

        while let Some(msg) = stream_domain.next().await {
            messages.push(msg);
        }

        // Should have 4 messages: 3 deltas + 1 completion
        assert_eq!(messages.len(), 4);

        // Verify deltas have content
        let delta1_msg = messages[0].as_ref().unwrap();
        assert_eq!(delta1_msg.content, Some(Content::part("<commit_message>")));

        let delta2_msg = messages[1].as_ref().unwrap();
        assert_eq!(
            delta2_msg.content,
            Some(Content::part("fix: avoid duplication"))
        );

        let delta3_msg = messages[2].as_ref().unwrap();
        assert_eq!(delta3_msg.content, Some(Content::part("</commit_message>")));

        // Completion event should have NO content (cleared to avoid duplication)
        let completion_msg = messages[3].as_ref().unwrap();
        assert_eq!(completion_msg.content, None);
        assert_eq!(completion_msg.finish_reason, Some(FinishReason::Stop));

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_avoids_duplicate_reasoning_in_completion() -> anyhow::Result<()> {
        // Simulate realistic streaming: reasoning deltas followed by completion event
        let reasoning_delta1 = fixture_delta_reasoning_text("Analyzing the request...");
        let reasoning_delta2 = fixture_delta_reasoning_text(" and formulating response.");
        let summary_delta = fixture_delta_reasoning_summary("Summary of analysis");

        // Completion event contains the full reasoning that was already streamed
        let response = fixture_response_with_reasoning_both(
            "Analyzing the request... and formulating response.",
            "Summary of analysis",
        );
        let completed = oai::ResponseCompletedEvent { sequence_number: 4, response };

        let stream: ResponseStream = Box::pin(tokio_stream::iter([
            event(oai::ResponseStreamEvent::ResponseReasoningTextDelta(
                reasoning_delta1,
            )),
            event(oai::ResponseStreamEvent::ResponseReasoningTextDelta(
                reasoning_delta2,
            )),
            event(oai::ResponseStreamEvent::ResponseReasoningSummaryTextDelta(
                summary_delta,
            )),
            event(oai::ResponseStreamEvent::ResponseCompleted(completed)),
        ]));

        let mut stream_domain = stream.into_domain()?;
        let mut messages: Vec<anyhow::Result<Message>> = vec![];

        while let Some(msg) = stream_domain.next().await {
            messages.push(msg);
        }

        // Should have 4 messages: 3 reasoning deltas + 1 completion
        assert_eq!(messages.len(), 4);

        // Verify reasoning deltas have reasoning content
        let delta1_msg = messages[0].as_ref().unwrap();
        assert_eq!(
            delta1_msg.reasoning,
            Some(Content::part("Analyzing the request..."))
        );
        assert!(delta1_msg.reasoning_details.is_some());

        let delta2_msg = messages[1].as_ref().unwrap();
        assert_eq!(
            delta2_msg.reasoning,
            Some(Content::part(" and formulating response."))
        );
        assert!(delta2_msg.reasoning_details.is_some());

        let summary_msg = messages[2].as_ref().unwrap();
        assert_eq!(
            summary_msg.reasoning,
            Some(Content::part("Summary of analysis"))
        );
        assert!(summary_msg.reasoning_details.is_some());

        // Completion event should have NO reasoning or reasoning_details (cleared to
        // avoid duplication)
        let completion_msg = messages[3].as_ref().unwrap();
        assert_eq!(completion_msg.reasoning, None);
        assert_eq!(completion_msg.reasoning_details, None);
        assert_eq!(completion_msg.finish_reason, Some(FinishReason::Stop));

        Ok(())
    }

    #[tokio::test]
    async fn test_stream_avoids_duplicate_tool_calls_in_completion() -> anyhow::Result<()> {
        // Simulate realistic streaming: tool call deltas followed by completion event
        let added = fixture_function_call_added("call_123", "shell", "");
        let delta1 = fixture_function_call_arguments_delta(0, r#"{"cmd":"echo"#);
        let delta2 = fixture_function_call_arguments_delta(0, r#" hello"}"#);

        // Completion event contains the full tool call that was already streamed
        let response =
            fixture_response_with_function_call("call_123", "shell", r#"{"cmd":"echo hello"}"#);
        let completed = oai::ResponseCompletedEvent { sequence_number: 4, response };

        let stream: ResponseStream = Box::pin(tokio_stream::iter([
            event(oai::ResponseStreamEvent::ResponseOutputItemAdded(added)),
            event(oai::ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(delta1)),
            event(oai::ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(delta2)),
            event(oai::ResponseStreamEvent::ResponseCompleted(completed)),
        ]));

        let mut stream_domain = stream.into_domain()?;
        let mut messages: Vec<anyhow::Result<Message>> = vec![];

        while let Some(msg) = stream_domain.next().await {
            messages.push(msg);
        }

        // Should have 3 messages: 2 tool call deltas + 1 completion
        assert_eq!(messages.len(), 3);

        // Verify tool call deltas have tool calls
        let delta1_msg = messages[0].as_ref().unwrap();
        assert_eq!(delta1_msg.tool_calls.len(), 1);

        let delta2_msg = messages[1].as_ref().unwrap();
        assert_eq!(delta2_msg.tool_calls.len(), 1);

        // Completion event should have NO tool calls (cleared to avoid duplication)
        let completion_msg = messages[2].as_ref().unwrap();
        assert_eq!(completion_msg.tool_calls.len(), 0);
        assert_eq!(completion_msg.finish_reason, Some(FinishReason::ToolCalls));

        Ok(())
    }

    // ============== ResponsesStreamEvent Tests ==============

    #[test]
    fn test_responses_stream_event_deserializes_keepalive() {
        let fixture = r#"{"type":"keepalive","sequence_number":3}"#;
        let actual: ResponsesStreamEvent = serde_json::from_str(fixture).unwrap();

        assert!(matches!(
            actual,
            ResponsesStreamEvent::Keepalive { sequence_number: 3 }
        ));
    }

    #[test]
    fn test_responses_stream_event_deserializes_response_event() {
        let fixture = serde_json::json!({
            "type": "response.output_text.delta",
            "sequence_number": 1,
            "item_id": "item_1",
            "output_index": 0,
            "content_index": 0,
            "delta": "hello"
        });
        let actual: ResponsesStreamEvent = serde_json::from_str(&fixture.to_string()).unwrap();

        assert!(matches!(actual, ResponsesStreamEvent::Response(_)));
    }

    #[test]
    fn test_responses_stream_event_ignores_unknown_type() {
        let fixture = r#"{"type":"totally_unknown_event","sequence_number":1}"#;
        let actual: ResponsesStreamEvent = serde_json::from_str(fixture).unwrap();

        assert!(matches!(actual, ResponsesStreamEvent::Unknown(_)));
    }

    #[test]
    fn test_responses_stream_event_deserializes_ping_with_cost() {
        let fixture = r#"{"type":"ping","cost":"0.00675010"}"#;
        let actual: ResponsesStreamEvent = serde_json::from_str(fixture).unwrap();

        match actual {
            ResponsesStreamEvent::Ping { cost } => {
                assert!((cost - 0.00675010).abs() < f64::EPSILON);
            }
            other => panic!("Expected Ping, got {:?}", other),
        }
    }

    #[test]
    fn test_responses_stream_event_deserializes_ping_with_numeric_cost() {
        let fixture = r#"{"type":"ping","cost":0.123}"#;
        let actual: ResponsesStreamEvent = serde_json::from_str(fixture).unwrap();

        match actual {
            ResponsesStreamEvent::Ping { cost } => {
                assert!((cost - 0.123).abs() < f64::EPSILON);
            }
            other => panic!("Expected Ping, got {:?}", other),
        }
    }

    /// Simulates the Spark model's streaming pattern: function call arguments
    /// are sent only in the `done` event (no deltas). The stream emits:
    /// 1. output_item.added (function_call with empty arguments)
    /// 2. function_call_arguments.done (complete arguments)
    /// 3. response.completed
    #[tokio::test]
    async fn test_spark_style_stream_function_call_no_deltas() -> anyhow::Result<()> {
        // Step 1: output_item.added with empty arguments (Spark sends "" initially)
        let added = fixture_function_call_added("call_shkZ0WZ4bgS2HdaAF0YOcB06", "shell", "");

        // Step 2: function_call_arguments.done with full arguments (no deltas)
        let done = oai::ResponseFunctionCallArgumentsDoneEvent {
            sequence_number: 5,
            output_index: 0,
            item_id: "fc_123".to_string(),
            name: Some("shell".to_string()),
            arguments: r#"{"command":"date \"+%Y-%m-%d\"","cwd":"/Users/amit/code-forge","description":"Get current date","env":[],"keep_ansi":false}"#.to_string(),
        };

        // Step 3: response.completed with usage
        let response: oai::Response = serde_json::from_value(serde_json::json!({
            "id": "resp_1",
            "created_at": 1773422509,
            "model": "gpt-5.3-codex-spark",
            "object": "response",
            "status": "completed",
            "output": [
                {
                    "type": "function_call",
                    "status": "completed",
                    "call_id": "call_shkZ0WZ4bgS2HdaAF0YOcB06",
                    "name": "shell",
                    "arguments": "{\"command\":\"date\"}"
                }
            ],
            "usage": {
                "input_tokens": 14900,
                "output_tokens": 381,
                "total_tokens": 15281,
                "input_tokens_details": { "cached_tokens": 14720 },
                "output_tokens_details": { "reasoning_tokens": 317 }
            }
        }))?;
        let completed = oai::ResponseCompletedEvent { sequence_number: 7, response };

        let stream: ResponseStream = Box::pin(tokio_stream::iter([
            event(oai::ResponseStreamEvent::ResponseOutputItemAdded(added)),
            event(oai::ResponseStreamEvent::ResponseFunctionCallArgumentsDone(
                done,
            )),
            event(oai::ResponseStreamEvent::ResponseCompleted(completed)),
        ]));

        let mut stream_domain = stream.into_domain()?;
        let mut messages = vec![];
        while let Some(msg) = stream_domain.next().await {
            messages.push(msg);
        }

        // Should get:
        // 1. Tool call from the done event (since no deltas were received)
        // 2. Completion metadata from response.completed
        assert_eq!(messages.len(), 2);

        // First message: tool call with full arguments
        let tool_msg = messages.remove(0)?;
        assert_eq!(tool_msg.tool_calls.len(), 1);
        let part = tool_msg.tool_calls[0].as_partial().unwrap();
        assert_eq!(
            part.call_id.as_ref().map(|id: &ToolCallId| id.as_str()),
            Some("call_shkZ0WZ4bgS2HdaAF0YOcB06")
        );
        assert_eq!(
            part.name.as_ref().map(|n: &ToolName| n.as_str()),
            Some("shell")
        );
        assert!(part.arguments_part.contains("\"command\""));

        // Second message: completion with usage and finish_reason
        let completion_msg = messages.remove(0)?;
        assert_eq!(completion_msg.finish_reason, Some(FinishReason::ToolCalls));
        assert!(completion_msg.usage.is_some());
        let usage = completion_msg.usage.unwrap();
        assert_eq!(usage.prompt_tokens, TokenCount::Actual(14900));
        assert_eq!(usage.completion_tokens, TokenCount::Actual(381));

        Ok(())
    }
}
