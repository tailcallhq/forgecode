use derive_more::derive::From;
use derive_setters::Setters;
use serde::{Deserialize, Serialize};
use strum_macros::{EnumString, IntoStaticStr};

use super::{ToolCall, ToolCallFull};
use crate::TokenCount;
use crate::reasoning::{Reasoning, ReasoningFull};

/// Labels an assistant message as intermediate commentary or the final answer.
///
/// For models like `gpt-5.3-codex` and beyond, when sending follow-up requests,
/// preserve and resend phase on all assistant messages -- dropping it can
/// degrade performance.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessagePhase {
    /// Intermediate commentary produced while the model is reasoning.
    Commentary,
    /// The final answer from the model.
    FinalAnswer,
}

#[derive(Default, Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    pub prompt_tokens: TokenCount,
    pub completion_tokens: TokenCount,
    pub total_tokens: TokenCount,
    pub cached_tokens: TokenCount,
    pub cost: Option<f64>,
}

impl Usage {
    /// Accumulates usage from another Usage instance by summing all fields.
    ///
    /// Use this for aggregating usage across **independent** requests (e.g.,
    /// session-level totals where each message has its own final usage).
    pub fn accumulate(mut self, other: &Usage) -> Self {
        self.prompt_tokens = self.prompt_tokens + other.prompt_tokens;
        self.completion_tokens = self.completion_tokens + other.completion_tokens;
        self.total_tokens = self.total_tokens + other.total_tokens;
        self.cached_tokens = self.cached_tokens + other.cached_tokens;
        self.cost = match (self.cost, other.cost) {
            (Some(a), Some(b)) => Some(a + b),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        self
    }

    /// Merges usage from another Usage instance using a "last non-zero wins"
    /// strategy.
    ///
    /// Use this when combining **partial** usage events within a single
    /// streaming response where values are **cumulative** (not incremental):
    /// - `message_start`: `input_tokens=1000, output_tokens=1`
    /// - `message_delta`:  `input_tokens=0,    output_tokens=75` (cumulative
    ///   total)
    ///
    /// For each field, the larger of the two values is kept. This prevents
    /// double-counting when providers report cumulative token counts across
    /// multiple events.
    ///
    /// Cost is summed since cost events are always additive.
    pub fn merge(mut self, other: &Usage) -> Self {
        self.prompt_tokens = self.prompt_tokens.max(other.prompt_tokens);
        self.completion_tokens = self.completion_tokens.max(other.completion_tokens);
        self.total_tokens = self.total_tokens.max(other.total_tokens);
        self.cached_tokens = self.cached_tokens.max(other.cached_tokens);
        self.cost = match (self.cost, other.cost) {
            (Some(a), Some(b)) => Some(a + b),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        self
    }
}

/// Represents a message that was received from the LLM provider
/// NOTE: Tool call messages are part of the larger Response object and not part
/// of the message.
#[derive(Default, Clone, Debug, Setters, PartialEq)]
#[setters(into, strip_option)]
pub struct ChatCompletionMessage {
    pub content: Option<Content>,
    pub thought_signature: Option<String>,
    pub reasoning: Option<Content>,
    pub reasoning_details: Option<Vec<Reasoning>>,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<FinishReason>,
    pub usage: Option<Usage>,
    /// Phase label for assistant messages (e.g. `Commentary` or `FinalAnswer`).
    /// Preserved from the response and replayed back on subsequent requests.
    pub phase: Option<MessagePhase>,
}

impl From<FinishReason> for ChatCompletionMessage {
    fn from(value: FinishReason) -> Self {
        ChatCompletionMessage::default().finish_reason(value)
    }
}

/// Represents partial or full content of a message
#[derive(Clone, Debug, PartialEq, Eq, From)]
pub enum Content {
    Part(ContentPart),
    Full(ContentFull),
}

impl Content {
    pub fn as_str(&self) -> &str {
        match self {
            Content::Part(part) => &part.0,
            Content::Full(full) => &full.0,
        }
    }

    pub fn part(content: impl ToString) -> Self {
        Content::Part(ContentPart(content.to_string()))
    }

    pub fn full(content: impl ToString) -> Self {
        Content::Full(ContentFull(content.to_string()))
    }

    pub fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }

    pub fn is_part(&self) -> bool {
        matches!(self, Content::Part(_))
    }
}

/// Used typically when streaming is enabled
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentPart(String);

/// Used typically when full responses are enabled (Streaming is disabled)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentFull(String);

impl<T: AsRef<str>> From<T> for Content {
    fn from(value: T) -> Self {
        Content::Full(ContentFull(value.as_ref().to_string()))
    }
}

/// The reason why the model stopped generating output.
/// Read more: https://platform.openai.com/docs/guides/function-calling#edge-cases
#[derive(Clone, Debug, Deserialize, Serialize, EnumString, IntoStaticStr, PartialEq, Eq)]
pub enum FinishReason {
    /// The model stopped generating output because it reached the maximum
    /// allowed length.
    #[strum(serialize = "length")]
    Length,
    /// The model stopped generating output because it encountered content that
    /// violated filters.
    #[strum(serialize = "content_filter")]
    ContentFilter,
    /// The model stopped generating output because it made a tool call.
    #[strum(serialize = "tool_calls")]
    ToolCalls,
    /// The model stopped generating output normally.
    #[strum(serialize = "stop", serialize = "end_turn")]
    Stop,
}

impl ChatCompletionMessage {
    pub fn assistant(content: impl Into<Content>) -> ChatCompletionMessage {
        ChatCompletionMessage::default().content(content.into())
    }

    pub fn add_reasoning_detail(mut self, detail: impl Into<Reasoning>) -> Self {
        let detail = detail.into();
        if let Some(ref mut details) = self.reasoning_details {
            details.push(detail);
        } else {
            self.reasoning_details = Some(vec![detail]);
        }
        self
    }

    pub fn add_tool_call(mut self, call_tool: impl Into<ToolCall>) -> Self {
        self.tool_calls.push(call_tool.into());
        self
    }

    pub fn extend_calls(mut self, calls: Vec<impl Into<ToolCall>>) -> Self {
        self.tool_calls.extend(calls.into_iter().map(Into::into));
        self
    }

    pub fn finish_reason_opt(mut self, reason: Option<FinishReason>) -> Self {
        self.finish_reason = reason;
        self
    }

    pub fn content_part(mut self, content: impl ToString) -> Self {
        self.content = Some(Content::Part(ContentPart(content.to_string())));
        self
    }

    pub fn content_full(mut self, content: impl ToString) -> Self {
        self.content = Some(Content::Full(ContentFull(content.to_string())));
        self
    }
}

/// Represents a complete message from the LLM provider with all content
/// collected This is typically used after processing a stream of
/// ChatCompletionMessage
#[derive(Clone, Debug, PartialEq)]
pub struct ChatCompletionMessageFull {
    pub content: String,
    pub thought_signature: Option<String>,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<ToolCallFull>,
    pub reasoning_details: Option<Vec<ReasoningFull>>,
    pub usage: Usage,
    pub finish_reason: Option<FinishReason>,
    /// Phase label for the assistant message (e.g. `Commentary` or
    /// `FinalAnswer`).
    pub phase: Option<MessagePhase>,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use pretty_assertions::assert_eq;

    use super::*;
    #[test]
    fn test_usage_accumulate_with_both_costs() {
        let fixture_usage_1 = Usage {
            prompt_tokens: TokenCount::Actual(100),
            completion_tokens: TokenCount::Actual(50),
            total_tokens: TokenCount::Actual(150),
            cached_tokens: TokenCount::Actual(20),
            cost: Some(0.01),
        };

        let fixture_usage_2 = Usage {
            prompt_tokens: TokenCount::Actual(200),
            completion_tokens: TokenCount::Actual(75),
            total_tokens: TokenCount::Actual(275),
            cached_tokens: TokenCount::Actual(30),
            cost: Some(0.02),
        };

        let actual = fixture_usage_1.accumulate(&fixture_usage_2);

        let expected = Usage {
            prompt_tokens: TokenCount::Actual(300),
            completion_tokens: TokenCount::Actual(125),
            total_tokens: TokenCount::Actual(425),
            cached_tokens: TokenCount::Actual(50),
            cost: Some(0.03),
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_usage_accumulate_mixed_token_types() {
        let fixture_usage_1 = Usage {
            prompt_tokens: TokenCount::Actual(100),
            completion_tokens: TokenCount::Approx(50),
            total_tokens: TokenCount::Actual(150),
            cached_tokens: TokenCount::Actual(20),
            cost: Some(0.01),
        };

        let fixture_usage_2 = Usage {
            prompt_tokens: TokenCount::Approx(200),
            completion_tokens: TokenCount::Actual(75),
            total_tokens: TokenCount::Approx(275),
            cached_tokens: TokenCount::Approx(30),
            cost: Some(0.02),
        };

        let actual = fixture_usage_1.accumulate(&fixture_usage_2);

        let expected = Usage {
            prompt_tokens: TokenCount::Approx(300),
            completion_tokens: TokenCount::Approx(125),
            total_tokens: TokenCount::Approx(425),
            cached_tokens: TokenCount::Approx(50),
            cost: Some(0.03),
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_usage_accumulate_partial_costs() {
        let fixture_usage_1 = Usage {
            prompt_tokens: TokenCount::Actual(100),
            completion_tokens: TokenCount::Actual(50),
            total_tokens: TokenCount::Actual(150),
            cached_tokens: TokenCount::Actual(20),
            cost: Some(0.01),
        };

        let fixture_usage_2 = Usage {
            prompt_tokens: TokenCount::Actual(200),
            completion_tokens: TokenCount::Actual(75),
            total_tokens: TokenCount::Actual(275),
            cached_tokens: TokenCount::Actual(30),
            cost: None,
        };

        let actual = fixture_usage_1.accumulate(&fixture_usage_2);

        let expected = Usage {
            prompt_tokens: TokenCount::Actual(300),
            completion_tokens: TokenCount::Actual(125),
            total_tokens: TokenCount::Actual(425),
            cached_tokens: TokenCount::Actual(50),
            cost: Some(0.01),
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_usage_accumulate_no_costs() {
        let fixture_usage_1 = Usage {
            prompt_tokens: TokenCount::Actual(100),
            completion_tokens: TokenCount::Actual(50),
            total_tokens: TokenCount::Actual(150),
            cached_tokens: TokenCount::Actual(20),
            cost: None,
        };

        let fixture_usage_2 = Usage {
            prompt_tokens: TokenCount::Actual(200),
            completion_tokens: TokenCount::Actual(75),
            total_tokens: TokenCount::Actual(275),
            cached_tokens: TokenCount::Actual(30),
            cost: None,
        };

        let actual = fixture_usage_1.accumulate(&fixture_usage_2);

        let expected = Usage {
            prompt_tokens: TokenCount::Actual(300),
            completion_tokens: TokenCount::Actual(125),
            total_tokens: TokenCount::Actual(425),
            cached_tokens: TokenCount::Actual(50),
            cost: None,
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_usage_accumulate_with_defaults() {
        let fixture_usage_1 = Usage::default();

        let fixture_usage_2 = Usage {
            prompt_tokens: TokenCount::Actual(200),
            completion_tokens: TokenCount::Actual(75),
            total_tokens: TokenCount::Actual(275),
            cached_tokens: TokenCount::Actual(30),
            cost: Some(0.05),
        };

        let actual = fixture_usage_1.accumulate(&fixture_usage_2);

        let expected = Usage {
            prompt_tokens: TokenCount::Actual(200),
            completion_tokens: TokenCount::Actual(75),
            total_tokens: TokenCount::Actual(275),
            cached_tokens: TokenCount::Actual(30),
            cost: Some(0.05),
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_finish_reason_from_str() {
        assert_eq!(
            FinishReason::from_str("length").unwrap(),
            FinishReason::Length
        );
        assert_eq!(
            FinishReason::from_str("content_filter").unwrap(),
            FinishReason::ContentFilter
        );
        assert_eq!(
            FinishReason::from_str("tool_calls").unwrap(),
            FinishReason::ToolCalls
        );
        assert_eq!(FinishReason::from_str("stop").unwrap(), FinishReason::Stop);
        assert_eq!(
            FinishReason::from_str("end_turn").unwrap(),
            FinishReason::Stop
        );
    }

    #[test]
    fn test_usage_merge_anthropic_cumulative() {
        // Fixture: Simulates Anthropic's message_start + message_delta pattern
        // where output_tokens in message_delta is CUMULATIVE (total), not a delta.
        let fixture_message_start = Usage {
            prompt_tokens: TokenCount::Actual(1000),
            completion_tokens: TokenCount::Actual(1), // Initial output token
            total_tokens: TokenCount::Actual(1001),
            cached_tokens: TokenCount::Actual(300),
            cost: None,
        };

        let fixture_message_delta = Usage {
            prompt_tokens: TokenCount::Actual(0),
            completion_tokens: TokenCount::Actual(75), // Cumulative total, NOT delta
            total_tokens: TokenCount::Actual(75),
            cached_tokens: TokenCount::Actual(0),
            cost: None,
        };

        let actual = fixture_message_start.merge(&fixture_message_delta);

        let expected = Usage {
            prompt_tokens: TokenCount::Actual(1000),   // max(1000, 0)
            completion_tokens: TokenCount::Actual(75), // max(1, 75) = 75, NOT 1+75=76
            total_tokens: TokenCount::Actual(1001),    // max(1001, 75)
            cached_tokens: TokenCount::Actual(300),    // max(300, 0)
            cost: None,
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_usage_merge_preserves_costs() {
        let fixture_usage_1 = Usage {
            prompt_tokens: TokenCount::Actual(100),
            completion_tokens: TokenCount::Actual(0),
            total_tokens: TokenCount::Actual(100),
            cached_tokens: TokenCount::Actual(0),
            cost: Some(0.01),
        };

        let fixture_usage_2 = Usage {
            prompt_tokens: TokenCount::Actual(0),
            completion_tokens: TokenCount::Actual(50),
            total_tokens: TokenCount::Actual(50),
            cached_tokens: TokenCount::Actual(0),
            cost: Some(0.02),
        };

        let actual = fixture_usage_1.merge(&fixture_usage_2);

        let expected = Usage {
            prompt_tokens: TokenCount::Actual(100),
            completion_tokens: TokenCount::Actual(50),
            total_tokens: TokenCount::Actual(100),
            cached_tokens: TokenCount::Actual(0),
            cost: Some(0.03), // Costs are summed, not maxed
        };

        assert_eq!(actual, expected);
    }
}
