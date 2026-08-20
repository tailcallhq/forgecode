use std::str::FromStr;

use forge_domain::{
    ChatCompletionMessage, Content, FinishReason, TokenCount, ToolCallFull, ToolCallId,
    ToolCallPart, ToolName, Usage,
};
use serde::{Deserialize, Serialize};

use super::tool_choice::FunctionType;
use crate::dto::openai::ReasoningDetail;
use crate::dto::openai::error::{Error, ErrorCode, ErrorResponse};

/// Represents a value that may be either a JSON number or a numeric string.
#[derive(Deserialize, Debug, Clone, PartialEq, derive_more::TryInto, Serialize)]
#[serde(untagged)]
pub enum StringOrF64 {
    Number(f64),
    String(String),
}

/// Epsilon for floating point comparison to handle near-zero costs
const COST_EPSILON: f64 = 1e-9;

/// Checks if a cost value is non-zero considering floating point precision
#[inline]
fn is_non_zero_cost(cost: &f64) -> bool {
    cost.abs() > COST_EPSILON
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum Response {
    Success {
        id: String,
        provider: Option<String>,
        #[serde(default)]
        model: Option<String>,
        choices: Vec<Choice>,
        #[serde(default)]
        created: u64,
        object: Option<String>,
        system_fingerprint: Option<String>,
        usage: Option<ResponseUsage>,
        #[serde(default)]
        prompt_filter_results: Option<Vec<PromptFilterResult>>,
    },
    CostOnly {
        choices: Vec<Choice>,
        cost: Option<StringOrF64>,
    },
    Failure {
        error: ErrorResponse,
    },
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PromptFilterResult {
    pub prompt_index: u32,
    pub content_filter_results: ContentFilterResults,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ContentFilterResults {
    pub hate: Option<FilterResult>,
    pub self_harm: Option<FilterResult>,
    pub sexual: Option<FilterResult>,
    pub violence: Option<FilterResult>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FilterResult {
    pub filtered: bool,
    pub severity: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ResponseUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
    pub cost: Option<f64>,
    pub prompt_tokens_details: Option<PromptTokenDetails>,
    pub cost_details: Option<CostDetails>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CostDetails {
    pub upstream_inference_cost: Option<f64>,
    pub upstream_inference_prompt_cost: Option<f64>,
    pub upstream_inference_completions_cost: Option<f64>,
}

impl CostDetails {
    fn total_cost(&self) -> Option<f64> {
        self.upstream_inference_cost
            .filter(is_non_zero_cost)
            .or({
                match (
                    self.upstream_inference_prompt_cost,
                    self.upstream_inference_completions_cost,
                ) {
                    (None, None) => None,
                    (Some(p), None) => Some(p),
                    (None, Some(c)) => Some(c),
                    (Some(p), Some(c)) => Some(p + c),
                }
            })
            .filter(is_non_zero_cost)
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PromptTokenDetails {
    pub cached_tokens: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum Choice {
    NonChat {
        finish_reason: Option<String>,
        text: String,
        error: Option<ErrorResponse>,
    },
    NonStreaming {
        logprobs: Option<serde_json::Value>,
        index: u32,
        finish_reason: Option<String>,
        message: ResponseMessage,
        error: Option<ErrorResponse>,
    },
    Streaming {
        finish_reason: Option<String>,
        delta: ResponseMessage,
        error: Option<ErrorResponse>,
    },
}

/// A message returned by a provider, used for both streaming deltas and
/// non-streaming responses.
///
/// `reasoning` and `reasoning_content` are kept as separate private fields
/// because some providers (e.g. `moonshotai/Kimi-K2.5-TEE`) emit **both**
/// keys in the same delta object. Using `#[serde(alias)]` would cause a
/// `duplicate_field` error in that case. Use [`ResponseMessage::reasoning`]
/// to read the value in preference order.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ResponseMessage {
    pub content: Option<String>,
    // Private: some providers (e.g. moonshotai/Kimi-K2.5-TEE) emit both keys
    // in the same delta object. Exposing them directly would let callers
    // accidentally read only one and miss the other. Use `reasoning()` instead,
    // which merges them in preference order.
    reasoning: Option<String>,
    reasoning_content: Option<String>,
    pub role: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub refusal: Option<String>,
    pub reasoning_details: Option<Vec<ReasoningDetail>>,
    // GitHub Copilot format (flat fields instead of array)
    pub reasoning_text: Option<String>,
    pub reasoning_opaque: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_content: Option<ExtraContent>,
}

impl ResponseMessage {
    /// Returns the reasoning text. When both `reasoning` and
    /// `reasoning_content` are present, the longer non-empty value is
    /// returned; otherwise whichever is non-empty is used.
    pub fn reasoning(&self) -> Option<&str> {
        match (self.reasoning.as_deref(), self.reasoning_content.as_deref()) {
            (Some(a), Some(b)) => {
                let a = a.trim();
                let b = b.trim();
                match (a.is_empty(), b.is_empty()) {
                    (true, _) => Some(b).filter(|s| !s.is_empty()),
                    (_, true) => Some(a).filter(|s| !s.is_empty()),
                    _ => Some(if b.len() > a.len() { b } else { a }),
                }
            }
            (Some(a), None) => Some(a).filter(|s| !s.trim().is_empty()),
            (None, Some(b)) => Some(b).filter(|s| !s.trim().is_empty()),
            (None, None) => None,
        }
    }
}

impl From<ReasoningDetail> for forge_domain::ReasoningDetail {
    fn from(detail: ReasoningDetail) -> Self {
        forge_domain::ReasoningDetail {
            text: detail.text,
            signature: detail.signature,
            data: detail.data,
            id: detail.id,
            format: detail.format,
            index: detail.index,
            type_of: Some(detail.r#type),
        }
    }
}

/// Google-specific metadata for Vertex AI thought signatures
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GoogleMetadata {
    pub thought_signature: Option<String>,
}

/// Extra content that may be included by certain providers (e.g., Vertex AI)
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ExtraContent {
    pub google: Option<GoogleMetadata>,
}

impl ExtraContent {
    /// Extracts the thought_signature from the extra content if present
    pub fn thought_signature(&self) -> Option<String> {
        self.google
            .as_ref()
            .and_then(|g| g.thought_signature.clone())
    }
}

impl From<String> for ExtraContent {
    fn from(thought_signature: String) -> Self {
        Self {
            google: Some(GoogleMetadata { thought_signature: Some(thought_signature) }),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ToolCall {
    pub id: Option<ToolCallId>,
    pub r#type: FunctionType,
    pub function: FunctionCall,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_content: Option<ExtraContent>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FunctionCall {
    // Only the first event typically has the name of the function call
    pub name: Option<ToolName>,
    #[serde(default)]
    pub arguments: String,
}

impl From<ResponseUsage> for Usage {
    fn from(usage: ResponseUsage) -> Self {
        let cost = usage
            .cost
            .filter(is_non_zero_cost)
            .or_else(|| {
                usage
                    .cost_details
                    .as_ref()
                    .and_then(CostDetails::total_cost)
            })
            .filter(is_non_zero_cost);

        Usage {
            prompt_tokens: TokenCount::Actual(usage.prompt_tokens),
            completion_tokens: TokenCount::Actual(usage.completion_tokens),
            total_tokens: TokenCount::Actual(usage.total_tokens),
            cached_tokens: usage
                .prompt_tokens_details
                .map(|token_details| TokenCount::Actual(token_details.cached_tokens))
                .unwrap_or_default(),
            cost,
        }
    }
}

/// Intermediate representation of GitHub Copilot reasoning fields
struct GitHubCopilotReasoning {
    text: Option<String>,
    data: Option<String>,
    r#type: Option<String>,
}

impl GitHubCopilotReasoning {
    fn into_reasoning_detail(self) -> forge_domain::ReasoningDetail {
        forge_domain::ReasoningDetail {
            text: self.text,
            data: self.data,
            type_of: self.r#type,
            ..Default::default()
        }
    }
}

/// Converts GitHub Copilot flat reasoning fields to structured reasoning
/// details
fn convert_github_copilot_reasoning(
    reasoning_text: &Option<String>,
    reasoning_opaque: &Option<String>,
) -> Option<Vec<GitHubCopilotReasoning>> {
    if reasoning_text.is_some() || reasoning_opaque.is_some() {
        let mut details = Vec::new();
        if let Some(text) = reasoning_text {
            details.push(GitHubCopilotReasoning {
                text: Some(text.clone()),
                data: None,
                r#type: Some("reasoning.text".to_string()),
            });
        }
        if let Some(opaque) = reasoning_opaque {
            details.push(GitHubCopilotReasoning {
                text: None,
                data: Some(opaque.clone()),
                r#type: Some("reasoning.encrypted".to_string()),
            });
        }
        Some(details)
    } else {
        None
    }
}

impl TryFrom<Response> for ChatCompletionMessage {
    type Error = anyhow::Error;

    fn try_from(res: Response) -> Result<Self, Self::Error> {
        match res {
            Response::Success { choices, usage, prompt_filter_results, .. } => {
                if let Some(choice) = choices.first() {
                    // Check if the choice has an error first
                    let error = match choice {
                        Choice::NonChat { error, .. } => error,
                        Choice::NonStreaming { error, .. } => error,
                        Choice::Streaming { error, .. } => error,
                    };

                    if let Some(error) = error {
                        return Err(Error::Response(Box::new(error.clone())).into());
                    }

                    let mut response = match choice {
                        Choice::NonChat { text, finish_reason, .. } => {
                            ChatCompletionMessage::assistant(Content::full(text)).finish_reason_opt(
                                finish_reason
                                    .clone()
                                    .and_then(|s| FinishReason::from_str(&s).ok()),
                            )
                        }
                        Choice::NonStreaming { message, finish_reason, .. } => {
                            let mut resp = ChatCompletionMessage::assistant(Content::full(
                                message.content.clone().unwrap_or_default(),
                            ))
                            .finish_reason_opt(
                                finish_reason
                                    .clone()
                                    .and_then(|s| FinishReason::from_str(&s).ok()),
                            );
                            if let Some(reasoning) = message.reasoning() {
                                resp = resp.reasoning(Content::full(reasoning.to_owned()));
                            }

                            if let Some(thought_signature) = message
                                .extra_content
                                .as_ref()
                                .and_then(ExtraContent::thought_signature)
                            {
                                resp = resp.thought_signature(thought_signature);
                            }

                            if let Some(reasoning_details) = &message.reasoning_details {
                                let converted_details: Vec<forge_domain::ReasoningFull> =
                                    reasoning_details
                                        .clone()
                                        .into_iter()
                                        .map(forge_domain::ReasoningFull::from)
                                        .collect();

                                resp = resp.add_reasoning_detail(forge_domain::Reasoning::Full(
                                    converted_details,
                                ));
                            } else if let Some(details) = convert_github_copilot_reasoning(
                                &message.reasoning_text,
                                &message.reasoning_opaque,
                            ) {
                                resp = resp.add_reasoning_detail(forge_domain::Reasoning::Full(
                                    details
                                        .into_iter()
                                        .map(GitHubCopilotReasoning::into_reasoning_detail)
                                        .collect(),
                                ));
                            }

                            if let Some(tool_calls) = &message.tool_calls {
                                for tool_call in tool_calls {
                                    let thought_signature = tool_call
                                        .extra_content
                                        .as_ref()
                                        .and_then(ExtraContent::thought_signature);

                                    resp = resp.add_tool_call(ToolCallFull {
                                        call_id: tool_call.id.clone(),
                                        name: tool_call
                                            .function
                                            .name
                                            .clone()
                                            .ok_or(forge_domain::Error::ToolCallMissingName)?,
                                        arguments: serde_json::from_str(
                                            &tool_call.function.arguments,
                                        )?,
                                        thought_signature,
                                    });
                                }
                            }
                            resp
                        }
                        Choice::Streaming { delta, finish_reason, .. } => {
                            let mut resp = ChatCompletionMessage::assistant(Content::part(
                                delta.content.clone().unwrap_or_default(),
                            ))
                            .finish_reason_opt(
                                finish_reason
                                    .clone()
                                    .and_then(|s| FinishReason::from_str(&s).ok()),
                            );

                            if let Some(reasoning) = delta.reasoning() {
                                resp = resp.reasoning(Content::part(reasoning.to_owned()));
                            }

                            if let Some(thought_signature) = delta
                                .extra_content
                                .as_ref()
                                .and_then(ExtraContent::thought_signature)
                            {
                                resp = resp.thought_signature(thought_signature);
                            }

                            if let Some(reasoning_details) = &delta.reasoning_details {
                                let converted_details: Vec<forge_domain::ReasoningPart> =
                                    reasoning_details
                                        .clone()
                                        .into_iter()
                                        .map(forge_domain::ReasoningPart::from)
                                        .collect();
                                resp = resp.add_reasoning_detail(forge_domain::Reasoning::Part(
                                    converted_details,
                                ));
                            } else if let Some(details) = convert_github_copilot_reasoning(
                                &delta.reasoning_text,
                                &delta.reasoning_opaque,
                            ) {
                                resp = resp.add_reasoning_detail(forge_domain::Reasoning::Part(
                                    details
                                        .into_iter()
                                        .map(GitHubCopilotReasoning::into_reasoning_detail)
                                        .collect(),
                                ));
                            }

                            if let Some(tool_calls) = &delta.tool_calls {
                                for tool_call in tool_calls {
                                    let thought_signature = tool_call
                                        .extra_content
                                        .as_ref()
                                        .and_then(ExtraContent::thought_signature);

                                    resp = resp.add_tool_call(ToolCallPart {
                                        call_id: tool_call.id.clone(),
                                        name: tool_call.function.name.clone(),
                                        arguments_part: tool_call.function.arguments.clone(),
                                        thought_signature,
                                    });
                                }
                            }
                            resp
                        }
                    };

                    if let Some(usage) = usage {
                        response.usage = Some(usage.into());
                    }
                    Ok(response)
                } else {
                    // Check if content was filtered
                    if let Some(filter_results) = prompt_filter_results
                        && let Some(filter_result) = filter_results.first()
                    {
                        let filtered_categories: Vec<String> = [
                            filter_result
                                .content_filter_results
                                .hate
                                .as_ref()
                                .filter(|f| f.filtered)
                                .map(|_| "hate"),
                            filter_result
                                .content_filter_results
                                .self_harm
                                .as_ref()
                                .filter(|f| f.filtered)
                                .map(|_| "self_harm"),
                            filter_result
                                .content_filter_results
                                .sexual
                                .as_ref()
                                .filter(|f| f.filtered)
                                .map(|_| "sexual"),
                            filter_result
                                .content_filter_results
                                .violence
                                .as_ref()
                                .filter(|f| f.filtered)
                                .map(|_| "violence"),
                        ]
                        .into_iter()
                        .flatten()
                        .map(String::from)
                        .collect();

                        if !filtered_categories.is_empty() {
                            let error = ErrorResponse::default()
                                .message(format!(
                                    "Content was filtered due to: {}",
                                    filtered_categories.join(", ")
                                ))
                                .code(ErrorCode::String("content_filter".to_string()));
                            return Err(Error::Response(Box::new(error)).into());
                        }
                    }

                    let mut default_response = ChatCompletionMessage::assistant(Content::full(""));
                    // No choices – this can happen with Ollama/LMStudio streaming where the final
                    // chunk only contains usage information.
                    if let Some(u) = usage {
                        default_response.usage = Some(u.into());
                    }
                    Ok(default_response)
                }
            }
            Response::CostOnly { cost, .. } => {
                let mut msg = ChatCompletionMessage::default();
                if let Some(c) = cost {
                    let cost_value = match c {
                        StringOrF64::Number(n) => n,
                        StringOrF64::String(s) => s.parse().unwrap_or(0.0),
                    };
                    msg.usage = Some(Usage {
                        prompt_tokens: TokenCount::Actual(0),
                        completion_tokens: TokenCount::Actual(0),
                        total_tokens: TokenCount::Actual(0),
                        cached_tokens: TokenCount::Actual(0),
                        cost: Some(cost_value),
                    });
                }
                Ok(msg)
            }
            Response::Failure { error } => Err(Error::Response(Box::new(error)).into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Context;
    use forge_domain::ChatCompletionMessage;

    use super::*;

    struct Fixture;

    fn response_message(
        reasoning: Option<&str>,
        reasoning_content: Option<&str>,
    ) -> ResponseMessage {
        ResponseMessage {
            content: None,
            reasoning: reasoning.map(str::to_owned),
            reasoning_content: reasoning_content.map(str::to_owned),
            role: None,
            tool_calls: None,
            refusal: None,
            reasoning_details: None,
            reasoning_text: None,
            reasoning_opaque: None,
            extra_content: None,
        }
    }

    #[test]
    fn test_reasoning_only_reasoning_field() {
        let fixture = response_message(Some("hello"), None);
        assert_eq!(fixture.reasoning(), Some("hello"));
    }

    #[test]
    fn test_reasoning_only_reasoning_content_field() {
        let fixture = response_message(None, Some("hello"));
        assert_eq!(fixture.reasoning(), Some("hello"));
    }

    #[test]
    fn test_reasoning_both_returns_longer() {
        let fixture = response_message(Some("short"), Some("much longer text"));
        assert_eq!(fixture.reasoning(), Some("much longer text"));
    }

    #[test]
    fn test_reasoning_both_equal_length_returns_reasoning() {
        let fixture = response_message(Some("aaa"), Some("bbb"));
        assert_eq!(fixture.reasoning(), Some("aaa"));
    }

    #[test]
    fn test_reasoning_both_present_one_empty_returns_non_empty() {
        let fixture = response_message(Some(""), Some("content"));
        assert_eq!(fixture.reasoning(), Some("content"));
    }

    #[test]
    fn test_reasoning_both_empty_returns_none() {
        let fixture = response_message(Some(""), Some(""));
        assert_eq!(fixture.reasoning(), None);
    }

    #[test]
    fn test_reasoning_neither_present_returns_none() {
        let fixture = response_message(None, None);
        assert_eq!(fixture.reasoning(), None);
    }

    async fn load_fixture(filename: &str) -> serde_json::Value {
        let fixture_path = format!("src/dto/openai/fixtures/{}", filename);
        let fixture_content = tokio::fs::read_to_string(&fixture_path)
            .await
            .unwrap_or_else(|_| panic!("Failed to read fixture file: {}", fixture_path));
        serde_json::from_str(&fixture_content)
            .unwrap_or_else(|_| panic!("Failed to parse JSON fixture: {}", fixture_path))
    }

    impl Fixture {
        // check if the response is compatible with the
        fn test_response_compatibility(message: &str) -> bool {
            let response = serde_json::from_str::<Response>(message)
                .with_context(|| format!("Failed to parse response: {message}"))
                .and_then(|event| {
                    ChatCompletionMessage::try_from(event.clone())
                        .with_context(|| "Failed to create completion message")
                });
            response.is_ok()
        }
    }

    #[test]
    fn test_open_ai_response_event() {
        let event = "{\"id\":\"chatcmpl-B2YVxGR9TaLBrEcFMVCv2B4IcNe4g\",\"object\":\"chat.completion.chunk\",\"created\":1739949029,\"model\":\"gpt-4o-mini-2024-07-18\",\"service_tier\":\"default\",\"system_fingerprint\":\"fp_00428b782a\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":null,\"tool_calls\":[{\"index\":0,\"id\":\"call_fmuXMsHhKD5eM2k0CvgNed53\",\"type\":\"function\",\"function\":{\"name\":\"shell\",\"arguments\":\"\"}}],\"refusal\":null},\"logprobs\":null,\"finish_reason\":null}]}";
        assert!(Fixture::test_response_compatibility(event));
    }

    #[test]
    fn test_forge_response_event() {
        let event = "{\"id\":\"gen-1739949430-JZMcABaj4fg8oFDtRNDZ\",\"provider\":\"OpenAI\",\"model\":\"openai/gpt-4o-mini\",\"object\":\"chat.completion.chunk\",\"created\":1739949430,\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":null,\"tool_calls\":[{\"index\":0,\"id\":\"call_bhjvz9w48ov4DSRhM15qLMmh\",\"type\":\"function\",\"function\":{\"name\":\"shell\",\"arguments\":\"\"}}],\"refusal\":null},\"logprobs\":null,\"finish_reason\":null,\"native_finish_reason\":null}],\"system_fingerprint\":\"fp_00428b782a\"}";
        assert!(Fixture::test_response_compatibility(event));
    }

    #[test]
    fn test_reasoning_response_event() {
        let event = "{\"id\":\"gen-1751626123-nYRpHzdA0thRXF0LoQi0\",\"provider\":\"Google\",\"model\":\"anthropic/claude-3.7-sonnet:thinking\",\"object\":\"chat.completion.chunk\",\"created\":1751626123,\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\",\"reasoning\":\"I need to check\",\"reasoning_details\":[{\"type\":\"reasoning.text\",\"text\":\"I need to check\"}]},\"finish_reason\":null,\"native_finish_reason\":null,\"logprobs\":null}]}";
        assert!(Fixture::test_response_compatibility(event));
    }

    #[tokio::test]
    async fn test_kimi_k2_both_reasoning_keys_event() {
        // moonshotai/Kimi-K2.5-TEE emits both "reasoning" and "reasoning_content"
        // in the same delta object. This must parse without a duplicate_field error.
        let fixture = load_fixture("chutes_completion_response.json").await;
        let actual = serde_json::from_value::<Response>(fixture);
        assert!(actual.is_ok(), "Failed to parse: {:?}", actual.err());
        let completion_result = ChatCompletionMessage::try_from(actual.unwrap());
        assert!(completion_result.is_ok());
    }

    #[test]
    fn test_fireworks_response_event_missing_arguments() {
        let event = "{\"id\":\"gen-1749331907-SttL6PXleVHnrdLMABfU\",\"provider\":\"Fireworks\",\"model\":\"qwen/qwen3-235b-a22b\",\"object\":\"chat.completion.chunk\",\"created\":1749331907,\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":null,\"tool_calls\":[{\"index\":0,\"id\":\"call_Wl2L8rrzHwrXSeiciIvU65IS\",\"type\":\"function\",\"function\":{\"name\":\"attempt_completion\"}}]},\"finish_reason\":null,\"native_finish_reason\":null,\"logprobs\":null}]}";
        assert!(Fixture::test_response_compatibility(event));
    }

    #[tokio::test]
    async fn test_responses() -> anyhow::Result<()> {
        let content = forge_test_kit::fixture!("/src/dto/openai/responses.jsonl").await;

        for (i, line) in content.split('\n').enumerate() {
            let i = i + 1;
            let _: Response = serde_json::from_str(line).with_context(|| {
                format!("Failed to parse response [responses.jsonl:{i}]: {line}")
            })?;
        }

        Ok(())
    }
    #[test]
    fn test_choice_error_handling_non_chat() {
        let error_response = ErrorResponse::default().message("Test error message".to_string());

        let response = Response::Success {
            id: "test-id".to_string(),
            provider: Some("test".to_string()),
            model: Some("test-model".to_string()),
            choices: vec![Choice::NonChat {
                text: "test content".to_string(),
                finish_reason: None,
                error: Some(error_response.clone()),
            }],
            created: 123456789,
            object: Some("chat.completion".to_string()),
            system_fingerprint: None,
            usage: None,
            prompt_filter_results: None,
        };

        let result = ChatCompletionMessage::try_from(response);
        assert!(result.is_err());
        let error = result.unwrap_err();
        let error_string = format!("{:?}", error);
        assert!(error_string.contains("Test error message"));
    }

    #[test]
    fn test_choice_error_handling_non_streaming() {
        let error_response = ErrorResponse::default().message("API limit exceeded".to_string());

        let response = Response::Success {
            id: "test-id".to_string(),
            provider: Some("test".to_string()),
            model: Some("test-model".to_string()),
            choices: vec![Choice::NonStreaming {
                logprobs: None,
                index: 0,
                finish_reason: None,
                message: ResponseMessage {
                    content: Some("test content".to_string()),
                    reasoning: None,
                    reasoning_content: None,
                    role: Some("assistant".to_string()),
                    tool_calls: None,
                    refusal: None,
                    reasoning_details: None,
                    reasoning_text: None,
                    reasoning_opaque: None,
                    extra_content: None,
                },
                error: Some(error_response.clone()),
            }],
            created: 123456789,
            object: Some("chat.completion".to_string()),
            system_fingerprint: None,
            usage: None,
            prompt_filter_results: None,
        };

        let result = ChatCompletionMessage::try_from(response);
        assert!(result.is_err());
        let error = result.unwrap_err();
        let error_string = format!("{:?}", error);
        assert!(error_string.contains("API limit exceeded"));
    }

    #[test]
    fn test_choice_error_handling_streaming() {
        let error_response = ErrorResponse::default().message("Stream interrupted".to_string());

        let response = Response::Success {
            id: "test-id".to_string(),
            provider: Some("test".to_string()),
            model: Some("test-model".to_string()),
            choices: vec![Choice::Streaming {
                finish_reason: None,
                delta: ResponseMessage {
                    content: Some("test content".to_string()),
                    reasoning: None,
                    reasoning_content: None,
                    role: Some("assistant".to_string()),
                    tool_calls: None,
                    refusal: None,
                    reasoning_details: None,
                    reasoning_text: None,
                    reasoning_opaque: None,
                    extra_content: None,
                },
                error: Some(error_response.clone()),
            }],
            created: 123456789,
            object: Some("chat.completion".to_string()),
            system_fingerprint: None,
            usage: None,
            prompt_filter_results: None,
        };

        let result = ChatCompletionMessage::try_from(response);
        assert!(result.is_err());
        let error = result.unwrap_err();
        let error_string = format!("{:?}", error);
        assert!(error_string.contains("Stream interrupted"));
    }

    #[test]
    fn test_choice_no_error_success() {
        let response = Response::Success {
            id: "test-id".to_string(),
            provider: Some("test".to_string()),
            model: Some("test-model".to_string()),
            choices: vec![Choice::NonStreaming {
                logprobs: None,
                index: 0,
                finish_reason: Some("stop".to_string()),
                message: ResponseMessage {
                    content: Some("Hello, world!".to_string()),
                    reasoning: None,
                    reasoning_content: None,
                    role: Some("assistant".to_string()),
                    tool_calls: None,
                    refusal: None,
                    reasoning_details: None,
                    reasoning_text: None,
                    reasoning_opaque: None,
                    extra_content: None,
                },
                error: None,
            }],
            created: 123456789,
            object: Some("chat.completion".to_string()),
            system_fingerprint: None,
            usage: None,
            prompt_filter_results: None,
        };

        let result = ChatCompletionMessage::try_from(response);
        assert!(result.is_ok());
        let message = result.unwrap();
        assert_eq!(message.content.unwrap().as_str(), "Hello, world!");
    }

    #[test]
    fn test_empty_choices_no_error() {
        let response = Response::Success {
            id: "test-id".to_string(),
            provider: Some("test".to_string()),
            model: Some("test-model".to_string()),
            choices: vec![],
            created: 123456789,
            object: Some("chat.completion".to_string()),
            system_fingerprint: None,
            usage: None,
            prompt_filter_results: None,
        };

        let result = ChatCompletionMessage::try_from(response);
        assert!(result.is_ok());
        let message = result.unwrap();
        assert_eq!(message.content.unwrap().as_str(), "");
    }

    #[test]
    fn test_cost_only_response_parses_and_returns_empty_message() {
        let fixture = r#"{"choices":[],"cost":"0"}"#;
        let actual = serde_json::from_str::<Response>(fixture).unwrap();

        let actual = ChatCompletionMessage::try_from(actual).unwrap();

        // CostOnly events now include the cost in the usage
        let expected = ChatCompletionMessage::default().usage(Usage {
            prompt_tokens: TokenCount::Actual(0),
            completion_tokens: TokenCount::Actual(0),
            total_tokens: TokenCount::Actual(0),
            cached_tokens: TokenCount::Actual(0),
            cost: Some(0.0),
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cost_only_response_numeric_cost_parses() {
        let fixture = r#"{"choices":[],"cost":0.0}"#;
        let actual = serde_json::from_str::<Response>(fixture);

        assert!(actual.is_ok());
    }

    #[tokio::test]
    async fn test_z_ai_response_compatibility() {
        let fixture = load_fixture("zai_api_delta_response.json").await;
        let actual = serde_json::from_value::<Response>(fixture);

        assert!(actual.is_ok());

        let response = actual.unwrap();
        let completion_result = ChatCompletionMessage::try_from(response);
        assert!(completion_result.is_ok());
    }

    #[tokio::test]
    async fn test_z_ai_response_complete_with_usage() {
        let fixture = load_fixture("zai_api_response.json").await;
        let actual = serde_json::from_value::<Response>(fixture);

        assert!(actual.is_ok());

        let response = actual.unwrap();
        let completion_result = ChatCompletionMessage::try_from(response);
        assert!(completion_result.is_ok());
    }

    #[test]
    fn test_response_usage_cost_priority_chain() {
        // Priority 1: cost field (non-zero) beats everything
        let fixture_cost_wins = ResponseUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            cost: Some(0.001),
            prompt_tokens_details: None,
            cost_details: Some(CostDetails {
                upstream_inference_cost: Some(0.005),
                upstream_inference_prompt_cost: Some(0.003),
                upstream_inference_completions_cost: Some(0.002),
            }),
        };

        let actual: Usage = fixture_cost_wins.into();
        assert_eq!(actual.cost, Some(0.001));

        // Priority 2: upstream_inference_cost beats partial costs
        let fixture_upstream_wins = ResponseUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            cost: None,
            prompt_tokens_details: None,
            cost_details: Some(CostDetails {
                upstream_inference_cost: Some(0.005),
                upstream_inference_prompt_cost: Some(0.003),
                upstream_inference_completions_cost: Some(0.002),
            }),
        };

        let actual: Usage = fixture_upstream_wins.into();
        assert_eq!(actual.cost, Some(0.005));

        // Priority 3: partial costs are summed when upstream_inference_cost is None
        let fixture_partial_sum = ResponseUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            cost: None,
            prompt_tokens_details: None,
            cost_details: Some(CostDetails {
                upstream_inference_cost: None,
                upstream_inference_prompt_cost: Some(0.003),
                upstream_inference_completions_cost: Some(0.002),
            }),
        };

        let actual: Usage = fixture_partial_sum.into();
        assert_eq!(actual.cost, Some(0.005));

        // Priority 4: when upstream_inference_cost is 0 then compute it from other
        // parameters.
        let fixture = ResponseUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            cost: None,
            prompt_tokens_details: None,
            cost_details: Some(CostDetails {
                upstream_inference_cost: Some(0.0),
                upstream_inference_prompt_cost: Some(0.003),
                upstream_inference_completions_cost: Some(0.002),
            }),
        };

        let actual: Usage = fixture.into();
        assert_eq!(actual.cost, Some(0.005));
    }

    #[test]
    fn test_zero_cost_should_fallback_to_cost_details() {
        let fixture = ResponseUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            cost: Some(0.0),
            prompt_tokens_details: None,
            cost_details: Some(CostDetails {
                upstream_inference_cost: Some(0.005),
                upstream_inference_prompt_cost: None,
                upstream_inference_completions_cost: None,
            }),
        };

        let actual: Usage = fixture.into();
        assert_eq!(actual.cost, Some(0.005));
    }

    #[test]
    fn test_near_zero_cost_should_fallback_to_cost_details() {
        let fixture = ResponseUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            cost: Some(1e-10),
            prompt_tokens_details: None,
            cost_details: Some(CostDetails {
                upstream_inference_cost: Some(0.005),
                upstream_inference_prompt_cost: None,
                upstream_inference_completions_cost: None,
            }),
        };

        let actual: Usage = fixture.into();
        assert_eq!(actual.cost, Some(0.005));
    }

    #[test]
    fn test_github_copilot_content_filter_response() {
        let response_json = r#"{
            "choices": [],
            "created": 0,
            "id": "",
            "prompt_filter_results": [{
                "content_filter_results": {
                    "hate": {"filtered": false, "severity": "safe"},
                    "self_harm": {"filtered": false, "severity": "safe"},
                    "sexual": {"filtered": false, "severity": "safe"},
                    "violence": {"filtered": false, "severity": "safe"}
                },
                "prompt_index": 0
            }]
        }"#;

        let actual = serde_json::from_str::<Response>(response_json);
        assert!(
            actual.is_ok(),
            "Should parse GitHub Copilot filter response: {:?}",
            actual.err()
        );
    }

    #[test]
    fn test_github_copilot_filtered_content_error() {
        let response = Response::Success {
            id: "".to_string(),
            provider: None,
            model: Some("gpt-5".to_string()),
            choices: vec![],
            created: 0,
            object: None,
            system_fingerprint: None,
            usage: None,
            prompt_filter_results: Some(vec![PromptFilterResult {
                prompt_index: 0,
                content_filter_results: ContentFilterResults {
                    hate: Some(FilterResult { filtered: true, severity: "high".to_string() }),
                    self_harm: Some(FilterResult { filtered: false, severity: "safe".to_string() }),
                    sexual: Some(FilterResult { filtered: false, severity: "safe".to_string() }),
                    violence: Some(FilterResult { filtered: false, severity: "safe".to_string() }),
                },
            }]),
        };

        let result = ChatCompletionMessage::try_from(response);
        assert!(result.is_err());
        let error = result.unwrap_err();
        let error_string = format!("{:?}", error);
        assert!(error_string.contains("Content was filtered"));
        assert!(error_string.contains("hate"));
    }

    #[test]
    fn test_nvidia_tool_call_streaming_chunk() {
        let response_json = r#"{"id":"chatcmpl-994182aa3bf1d873","object":"chat.completion.chunk","created":1775363363,"model":"qwen/qwen3.5-397b-a17b","choices":[{"index":0,"delta":{"content":null,"reasoning":null,"reasoning_content":null,"tool_calls":[{"index":1,"function":{"arguments":"}"}}]},"logprobs":null,"finish_reason":"tool_calls","stop_reason":null,"token_ids":null}]}"#;

        let actual = serde_json::from_str::<Response>(response_json);
        assert!(
            actual.is_ok(),
            "Should parse NVIDIA tool call streaming chunk: {:?}",
            actual.err()
        );
    }

    #[test]
    fn test_nvidia_tool_call_deserialization() {
        // NVIDIA sends tool calls without "id" and "type" fields
        let tool_call_json = r#"{"index":1,"function":{"arguments":"}"}}"#;
        let actual = serde_json::from_str::<ToolCall>(tool_call_json);
        assert!(
            actual.is_ok(),
            "Should parse NVIDIA tool call: {:?}",
            actual.err()
        );
    }

    #[test]
    fn test_nvidia_response_message_deserialization() {
        let msg_json = r#"{"content":null,"reasoning":null,"reasoning_content":null,"tool_calls":[{"index":1,"function":{"arguments":"}"}}]}"#;
        let actual = serde_json::from_str::<ResponseMessage>(msg_json);
        assert!(
            actual.is_ok(),
            "Should parse NVIDIA response message: {:?}",
            actual.err()
        );
    }

    #[test]
    fn test_nvidia_choice_deserialization() {
        let choice_json = r#"{"index":0,"delta":{"content":null,"reasoning":null,"reasoning_content":null,"tool_calls":[{"index":1,"function":{"arguments":"}"}}]},"logprobs":null,"finish_reason":"tool_calls","stop_reason":null,"token_ids":null}"#;
        let actual = serde_json::from_str::<Choice>(choice_json);
        assert!(
            actual.is_ok(),
            "Should parse NVIDIA choice: {:?}",
            actual.err()
        );
    }
}
