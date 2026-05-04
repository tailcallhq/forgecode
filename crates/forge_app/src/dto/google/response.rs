use forge_domain::{
    ChatCompletionMessage, FinishReason, Reasoning, ReasoningPart, TokenCount, ToolCallId,
    ToolCallPart, ToolName,
};
use serde::Deserialize;

/// Model information from Google API
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl From<Model> for forge_domain::Model {
    fn from(value: Model) -> Self {
        // Extract model ID from name (format: "models/gemini-2.0-flash")
        let id = value
            .name
            .strip_prefix("models/")
            .unwrap_or(&value.name)
            .to_string();

        // Determine context length based on model name
        let context_length = if id.contains("gemini-2.0") || id.contains("gemini-1.5") {
            2_000_000 // 2M tokens for Gemini 2.0 and 1.5
        } else {
            32_000 // Default for older models
        };

        forge_domain::Model {
            id: forge_domain::ModelId::new(id),
            name: Some(value.display_name.unwrap_or(value.name)),
            description: value.description,
            context_length: Some(context_length),
            tools_supported: Some(true), // Google models support function calling
            supports_parallel_tool_calls: Some(true),
            supports_reasoning: Some(true), // Gemini 2.0+ supports thinking
            input_modalities: vec![],       // Google supports text, images, audio, video
        }
    }
}

/// EventData for Google streaming responses
/// Google returns chunks directly without event wrappers
#[derive(Deserialize, PartialEq, Clone, Debug)]
#[serde(untagged)]
pub enum EventData {
    Response(Response),
    Error(ErrorResponse),
    Ping(PingEvent),
    Unknown(serde_json::Value),
}

/// Represents a value that may be either a JSON number or a numeric string,
/// used for fields like `cost` that proxies sometimes encode as strings.
#[derive(Deserialize, Debug, Clone, PartialEq, derive_more::TryInto)]
#[serde(untagged)]
pub enum StringOrF64 {
    Number(f64),
    String(String),
}

/// Heartbeat/cost event sent by some proxies (e.g. opencode.ai).
///
/// Example payload: `{"type":"ping","cost":"0.02889400"}`
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct PingEvent {
    pub cost: StringOrF64,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct ErrorResponse {
    pub error: ErrorContent,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct ErrorContent {
    pub code: i32,
    pub message: String,
    pub status: String,
}

impl TryFrom<EventData> for ChatCompletionMessage {
    type Error = anyhow::Error;

    fn try_from(value: EventData) -> Result<Self, Self::Error> {
        match value {
            EventData::Response(response) => ChatCompletionMessage::try_from(response),
            EventData::Error(e) => Err(anyhow::anyhow!(
                "Google API Error {}: {}",
                e.error.code,
                e.error.message
            )),
            EventData::Ping(ping) => {
                // Extract cost from proxy ping events (e.g. opencode.ai)
                let cost = match ping.cost {
                    StringOrF64::Number(n) => n,
                    StringOrF64::String(s) => s.parse().unwrap_or(0.0),
                };
                let usage = forge_domain::Usage { cost: Some(cost), ..Default::default() };
                Ok(ChatCompletionMessage::assistant(forge_domain::Content::part("")).usage(usage))
            }
            EventData::Unknown(_) => {
                // Silently ignore any other unrecognised events
                Ok(ChatCompletionMessage::assistant(
                    forge_domain::Content::part(""),
                ))
            }
        }
    }
}
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub candidates: Vec<Candidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_metadata: Option<UsageMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_feedback: Option<PromptFeedback>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_ratings: Option<Vec<SafetyRating>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grounding_metadata: Option<GroundingMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_context_metadata: Option<UrlContextMetadata>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Content {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<Part>>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum Part {
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: FunctionCall,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "thoughtSignature")]
        thought_signature: Option<String>,
    },
    InlineData {
        #[serde(rename = "inlineData")]
        inline_data: InlineData,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "thoughtSignature")]
        thought_signature: Option<String>,
    },
    Text {
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thought: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "thoughtSignature")]
        thought_signature: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "executableCode")]
        executable_code: Option<ExecutableCode>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "codeExecutionResult")]
        code_execution_result: Option<CodeExecutionResult>,
    },
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FunctionCall {
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InlineData {
    pub mime_type: String,
    pub data: String,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct ExecutableCode {
    pub language: String,
    pub code: String,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct CodeExecutionResult {
    pub outcome: String,
    pub output: String,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SafetyRating {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probability: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probability_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<bool>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_content_token_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thoughts_token_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_token_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates_token_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_token_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traffic_type: Option<String>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PromptFeedback {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_ratings: Option<Vec<SafetyRating>>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GroundingMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search_queries: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieval_queries: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_entry_point: Option<SearchEntryPoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grounding_chunks: Option<Vec<GroundingChunk>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grounding_supports: Option<Vec<GroundingSupport>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieval_metadata: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchEntryPoint {
    pub rendered_content: String,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct GroundingChunk {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web: Option<WebChunk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retrieved_context: Option<RetrievedContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maps: Option<MapsChunk>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct WebChunk {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RetrievedContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_search_store: Option<String>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MapsChunk {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place_id: Option<String>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GroundingSupport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment: Option<Segment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "segment_text")]
    pub segment_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grounding_chunk_indices: Option<Vec<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub support_chunk_indices: Option<Vec<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_scores: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_score: Option<Vec<f64>>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UrlContextMetadata {
    pub url_metadata: Vec<UrlMetadata>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UrlMetadata {
    pub retrieved_url: String,
    pub url_retrieval_status: String,
}

impl From<UsageMetadata> for forge_domain::Usage {
    fn from(usage: UsageMetadata) -> Self {
        let prompt_tokens = usage.prompt_token_count.unwrap_or_default() as usize;
        let completion_tokens = usage.candidates_token_count.unwrap_or_default() as usize;
        let cached_tokens = usage.cached_content_token_count.unwrap_or_default() as usize;
        let total_tokens = usage.total_token_count.unwrap_or_default() as usize;

        forge_domain::Usage {
            prompt_tokens: TokenCount::Actual(prompt_tokens),
            completion_tokens: TokenCount::Actual(completion_tokens),
            total_tokens: TokenCount::Actual(total_tokens),
            cached_tokens: TokenCount::Actual(cached_tokens),
            ..Default::default()
        }
    }
}

/// Converts Google's finish reason strings to domain FinishReason
fn parse_finish_reason(reason: &str) -> Option<FinishReason> {
    match reason {
        "STOP" => Some(FinishReason::Stop),
        "MAX_TOKENS" => Some(FinishReason::Length),
        "SAFETY" | "RECITATION" => Some(FinishReason::ContentFilter),
        _ => Some(FinishReason::Stop), // Default to Stop for unknown reasons
    }
}

impl TryFrom<Part> for ChatCompletionMessage {
    type Error = anyhow::Error;

    fn try_from(part: Part) -> Result<Self, Self::Error> {
        match part {
            Part::Text {
                text,
                thought,
                thought_signature,
                executable_code: _,
                code_execution_result: _,
            } => {
                let text_content = text.unwrap_or_default();
                let is_thought = thought.unwrap_or(false);

                if is_thought {
                    // This is a thinking/reasoning part
                    let mut msg = ChatCompletionMessage::assistant(forge_domain::Content::part(""))
                        .reasoning(forge_domain::Content::part(text_content.clone()))
                        .add_reasoning_detail(Reasoning::Part(vec![
                            ReasoningPart::default()
                                .text(Some(text_content))
                                .signature(thought_signature.clone()),
                        ]));

                    if let Some(signature) = thought_signature {
                        msg = msg.thought_signature(signature);
                    }

                    Ok(msg)
                } else {
                    // Regular text content
                    let mut msg =
                        ChatCompletionMessage::assistant(forge_domain::Content::part(text_content));
                    if let Some(signature) = thought_signature {
                        msg = msg.thought_signature(signature);
                    }
                    Ok(msg)
                }
            }
            Part::FunctionCall { function_call, thought_signature } => Ok(
                ChatCompletionMessage::assistant(forge_domain::Content::part("")).add_tool_call(
                    ToolCallPart {
                        call_id: Some(ToolCallId::generate()),
                        name: Some(ToolName::new(function_call.name)),
                        arguments_part: serde_json::to_string(&function_call.args)?,
                        thought_signature,
                        namespace: None,
                    },
                ),
            ),
            Part::InlineData { .. } => {
                // For now, skip inline data in responses (it's typically for inputs)
                Ok(ChatCompletionMessage::assistant(
                    forge_domain::Content::part(""),
                ))
            }
        }
    }
}

impl TryFrom<Candidate> for ChatCompletionMessage {
    type Error = anyhow::Error;

    fn try_from(candidate: Candidate) -> Result<Self, Self::Error> {
        let mut content_parts: Vec<String> = Vec::new();
        let mut reasoning_parts: Vec<String> = Vec::new();
        let mut tool_calls: Vec<forge_domain::ToolCall> = Vec::new();
        let mut reasoning_details: Option<Vec<forge_domain::Reasoning>> = None;
        let mut thought_signature: Option<String> = None;
        let mut finish_reason: Option<FinishReason> = None;

        // Add finish reason if present
        if let Some(ref reason) = candidate.finish_reason {
            finish_reason = parse_finish_reason(reason);
        }

        // Process content parts
        if let Some(content) = candidate.content
            && let Some(parts) = content.parts
        {
            for part in parts {
                let part_message = ChatCompletionMessage::try_from(part)?;

                // Collect content text
                if let Some(part_content) = part_message.content {
                    let text = part_content.as_str();
                    if !text.is_empty() {
                        content_parts.push(text.to_string());
                    }
                }

                // Collect reasoning text
                if let Some(part_reasoning) = part_message.reasoning {
                    let text = part_reasoning.as_str();
                    if !text.is_empty() {
                        reasoning_parts.push(text.to_string());
                    }
                }

                // Collect reasoning details (accumulate)
                if let Some(details) = part_message.reasoning_details {
                    if let Some(ref mut current) = reasoning_details {
                        current.extend(details);
                    } else {
                        reasoning_details = Some(details);
                    }
                }

                // Collect tool calls
                tool_calls.extend(part_message.tool_calls);

                // Take thought signature (last one wins)
                if part_message.thought_signature.is_some() {
                    thought_signature = part_message.thought_signature;
                }
            }
        }

        // Build the final message
        let content = content_parts.join("");
        let mut message = ChatCompletionMessage::assistant(forge_domain::Content::part(content));

        if let Some(finish) = finish_reason {
            message = message.finish_reason(finish);
        }

        if !reasoning_parts.is_empty() {
            message = message.reasoning(forge_domain::Content::part(reasoning_parts.join("")));
        }

        if let Some(details) = reasoning_details {
            message.reasoning_details = Some(details);
        }

        if !tool_calls.is_empty() {
            message.tool_calls = tool_calls;
        }

        if let Some(signature) = thought_signature {
            message = message.thought_signature(signature);
        }

        Ok(message)
    }
}

impl TryFrom<Response> for ChatCompletionMessage {
    type Error = anyhow::Error;

    fn try_from(response: Response) -> Result<Self, Self::Error> {
        // Get the first candidate
        if let Some(candidate) = response.candidates.into_iter().next() {
            let mut message = ChatCompletionMessage::try_from(candidate)?;

            // Add usage metadata if present
            if let Some(usage) = response.usage_metadata {
                message.usage = Some(usage.into());
            }

            Ok(message)
        } else {
            // No candidates - return empty message
            let mut message = ChatCompletionMessage::assistant(forge_domain::Content::part(""));

            // Still add usage if present
            if let Some(usage) = response.usage_metadata {
                message.usage = Some(usage.into());
            }

            Ok(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_chat_completion_message_from_part_function_call_generates_id() {
        let function_call =
            FunctionCall { name: "test_tool".to_string(), args: json!({"arg": "value"}) };

        let part = Part::FunctionCall { function_call, thought_signature: None };

        let message = ChatCompletionMessage::try_from(part).unwrap();

        assert!(!message.tool_calls.is_empty());
        let tool_calls = message.tool_calls;
        assert_eq!(tool_calls.len(), 1);

        let tool_call = &tool_calls[0];
        match tool_call {
            forge_domain::ToolCall::Part(part) => {
                assert!(part.call_id.is_some());
                let call_id = part.call_id.as_ref().unwrap();
                assert!(call_id.as_str().starts_with("forge_call_id_"));
            }
            _ => panic!("Expected ToolCall::Part"),
        }
    }

    #[test]
    fn test_model_conversion() {
        let model = Model {
            name: "models/gemini-pro".to_string(),
            display_name: Some("Gemini Pro".to_string()),
            description: Some("A model".to_string()),
        };
        let domain_model: forge_domain::Model = model.into();
        assert_eq!(domain_model.id.as_str(), "gemini-pro");
        assert_eq!(domain_model.name.unwrap(), "Gemini Pro");
        assert_eq!(domain_model.context_length.unwrap(), 32_000);

        let model_v2 = Model {
            name: "models/gemini-2.0-flash".to_string(),
            display_name: None,
            description: None,
        };
        let domain_model_v2: forge_domain::Model = model_v2.clone().into();
        assert_eq!(domain_model_v2.id.as_str(), "gemini-2.0-flash");
        assert_eq!(domain_model_v2.name.unwrap(), "models/gemini-2.0-flash");
        assert_eq!(domain_model_v2.context_length.unwrap(), 2_000_000);
    }

    #[test]
    fn test_ping_event_extracts_cost() {
        let fixture = json!({"type": "ping", "cost": "0.02889400"});
        let event_data: EventData = serde_json::from_value(fixture).unwrap();
        assert!(matches!(event_data, EventData::Ping(_)));

        let actual = ChatCompletionMessage::try_from(event_data).unwrap();
        let expected = ChatCompletionMessage::assistant(forge_domain::Content::part(""))
            .usage(forge_domain::Usage { cost: Some(0.028894), ..Default::default() });
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_ping_event_with_numeric_cost() {
        let fixture = json!({"type": "ping", "cost": 0.05});
        let event_data: EventData = serde_json::from_value(fixture).unwrap();
        assert!(matches!(event_data, EventData::Ping(_)));

        let actual = ChatCompletionMessage::try_from(event_data).unwrap();
        assert_eq!(actual.usage.unwrap().cost, Some(0.05));
    }

    #[test]
    fn test_unknown_event_returns_empty_message() {
        let fixture = json!({"type": "something_else", "data": 123});
        let event_data: EventData = serde_json::from_value(fixture).unwrap();
        assert!(matches!(event_data, EventData::Unknown(_)));

        let actual = ChatCompletionMessage::try_from(event_data).unwrap();
        let expected = ChatCompletionMessage::assistant(forge_domain::Content::part(""));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_event_data_parsing() {
        let response_json = json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello"}]
                }
            }]
        });
        let event_data: EventData = serde_json::from_value(response_json).unwrap();
        match event_data {
            EventData::Response(_) => {}
            _ => panic!("Expected Response"),
        }

        let error_json = json!({
            "error": {
                "code": 400,
                "message": "Bad Request",
                "status": "INVALID_ARGUMENT"
            }
        });
        let event_data_err: EventData = serde_json::from_value(error_json).unwrap();
        match event_data_err {
            EventData::Error(e) => {
                assert_eq!(e.error.code, 400);
                assert_eq!(e.error.message, "Bad Request");
            }
            _ => panic!("Expected Error"),
        }
    }

    #[test]
    fn test_candidate_to_message_conversion() {
        let candidate = Candidate {
            content: Some(Content {
                role: Some("model".to_string()),
                parts: Some(vec![
                    Part::Text {
                        text: Some("Hello".to_string()),
                        thought: None,
                        thought_signature: None,
                        executable_code: None,
                        code_execution_result: None,
                    },
                    Part::Text {
                        text: Some("Thinking...".to_string()),
                        thought: Some(true),
                        thought_signature: Some("sig123".to_string()),
                        executable_code: None,
                        code_execution_result: None,
                    },
                ]),
            }),
            finish_reason: Some("STOP".to_string()),
            safety_ratings: None,
            grounding_metadata: None,
            url_context_metadata: None,
        };

        let message = ChatCompletionMessage::try_from(candidate).unwrap();

        // Check content
        assert_eq!(message.content.unwrap().as_str(), "Hello");

        // Check reasoning
        assert_eq!(message.reasoning.unwrap().as_str(), "Thinking...");

        // Check finish reason
        assert_eq!(message.finish_reason.unwrap(), FinishReason::Stop);

        // Check thought signature
        assert_eq!(message.thought_signature.unwrap(), "sig123");
    }

    #[test]
    fn test_usage_metadata_conversion() {
        let usage = UsageMetadata {
            prompt_token_count: Some(10),
            candidates_token_count: Some(20),
            total_token_count: Some(30),
            cached_content_token_count: Some(5),
            thoughts_token_count: None,
            traffic_type: None,
        };

        let domain_usage: forge_domain::Usage = usage.into();

        assert_eq!(domain_usage.prompt_tokens, TokenCount::Actual(10));
        assert_eq!(domain_usage.completion_tokens, TokenCount::Actual(20));
        assert_eq!(domain_usage.total_tokens, TokenCount::Actual(30));
        assert_eq!(domain_usage.cached_tokens, TokenCount::Actual(5));
    }

    #[test]
    fn test_part_text_conversion() {
        let part = Part::Text {
            text: Some("Hello".to_string()),
            thought: None,
            thought_signature: None,
            executable_code: None,
            code_execution_result: None,
        };
        let msg = ChatCompletionMessage::try_from(part).unwrap();
        assert_eq!(msg.content.unwrap().as_str(), "Hello");
        assert!(msg.reasoning.is_none());

        // Test thought
        let part = Part::Text {
            text: Some("Thinking...".to_string()),
            thought: Some(true),
            thought_signature: Some("sig".to_string()),
            executable_code: None,
            code_execution_result: None,
        };
        let msg = ChatCompletionMessage::try_from(part).unwrap();
        assert_eq!(msg.content.unwrap().as_str(), ""); // Content should be empty for pure thought part
        assert_eq!(msg.reasoning.unwrap().as_str(), "Thinking...");
        assert_eq!(msg.thought_signature.unwrap(), "sig");
    }

    #[test]
    fn test_response_no_candidates() {
        let response = Response {
            candidates: vec![],
            usage_metadata: Some(UsageMetadata {
                prompt_token_count: Some(10),
                candidates_token_count: Some(20),
                total_token_count: Some(30),
                cached_content_token_count: None,
                thoughts_token_count: None,
                traffic_type: None,
            }),
            prompt_feedback: None,
        };

        let msg = ChatCompletionMessage::try_from(response).unwrap();
        assert_eq!(msg.content.unwrap().as_str(), "");

        let usage = msg.usage.unwrap();
        assert_eq!(usage.prompt_tokens, TokenCount::Actual(10));
        assert_eq!(usage.completion_tokens, TokenCount::Actual(20));
    }
}
