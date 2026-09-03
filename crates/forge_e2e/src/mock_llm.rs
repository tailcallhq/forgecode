//! Mock LLM provider that returns scripted responses.
//!
//! The mock accepts a queue of scripted responses and pops one off for each
//! chat-completion request. This makes tests deterministic and offline.

use crate::assertions::{ExpectedText, ExpectedTool};
use serde::{Deserialize, Serialize};

/// What the mock should return for one turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScriptedResponse {
    /// Plain assistant text, no tool calls.
    Text(ExpectedText),
    /// One or more tool calls, no accompanying text.
    Tools(Vec<ExpectedTool>),
    /// Both text and tool calls (e.g. "I'll read it" then a `read` call).
    TextAndTools {
        text: ExpectedText,
        tools: Vec<ExpectedTool>,
    },
}

impl ScriptedResponse {
    pub fn text_only(text: ExpectedText) -> Self {
        ScriptedResponse::Text(text)
    }

    pub fn tool_only(tool: ExpectedTool) -> Self {
        ScriptedResponse::Tools(vec![tool])
    }

    pub fn tool_only_many(tools: Vec<ExpectedTool>) -> Self {
        ScriptedResponse::Tools(tools)
    }

    pub fn text_then_tool(text: ExpectedText, tool: ExpectedTool) -> Self {
        ScriptedResponse::TextAndTools { text, tools: vec![tool] }
    }
}

/// Builder for a list of scripted responses.
#[derive(Debug, Clone, Default)]
pub struct MockLlm {
    responses: Vec<ScriptedResponse>,
    /// Whether to record every request the mock receives (for assertions).
    record_requests: bool,
}

impl MockLlm {
    pub fn new() -> Self {
        Self { responses: Vec::new(), record_requests: false }
    }

    /// Append a response to the script.
    pub fn then(mut self, resp: ScriptedResponse) -> Self {
        self.responses.push(resp);
        self
    }

    /// Convenience: append a text-only response.
    pub fn then_text(self, text: ExpectedText) -> Self {
        self.then(ScriptedResponse::text_only(text))
    }

    /// Convenience: append a tool-only response.
    pub fn then_tool(self, tool: ExpectedTool) -> Self {
        self.then(ScriptedResponse::tool_only(tool))
    }

    /// Convenience: text + tool response.
    pub fn then_text_and_tool(self, text: ExpectedText, tool: ExpectedTool) -> Self {
        self.then(ScriptedResponse::text_then_tool(text, tool))
    }

    pub fn with_request_recording(mut self) -> Self {
        self.record_requests = true;
        self
    }

    /// Number of remaining responses in the script.
    pub fn remaining(&self) -> usize {
        self.responses.len()
    }

    /// Pop the next response. Panics if the script is exhausted.
    pub fn next_response(&mut self) -> ScriptedResponse {
        if self.responses.is_empty() {
            panic!("mock_llm script exhausted; no more responses queued");
        }
        self.responses.remove(0)
    }

    /// Convenience constructor used in doctests.
    pub fn text_then_tool(text: ExpectedText, tool: ExpectedTool) -> ScriptedResponse {
        ScriptedResponse::text_then_tool(text, tool)
    }

    pub fn tool_only(tool: ExpectedTool) -> ScriptedResponse {
        ScriptedResponse::tool_only(tool)
    }

    pub fn text_only(text: ExpectedText) -> ScriptedResponse {
        ScriptedResponse::text_only(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_llm_queues_responses_in_order() {
        let mut mock = MockLlm::new()
            .then_text(ExpectedText::contains("hello"))
            .then_tool(ExpectedTool::new("read"));
        assert_eq!(mock.remaining(), 2);

        let first = mock.next_response();
        match first {
            ScriptedResponse::Text(_) => {}
            _ => panic!("expected text response first"),
        }

        let second = mock.next_response();
        match second {
            ScriptedResponse::Tools(_) => {}
            _ => panic!("expected tool response second"),
        }

        assert_eq!(mock.remaining(), 0);
    }

    #[test]
    #[should_panic(expected = "exhausted")]
    fn mock_llm_panics_when_exhausted() {
        let mut mock = MockLlm::new().then_text(ExpectedText::Empty);
        let _ = mock.next_response();
        let _ = mock.next_response();
    }
}
