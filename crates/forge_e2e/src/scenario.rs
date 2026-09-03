//! Scenario runner for end-to-end tests.
//!
//! A `Scenario` is a declarative script:
//!
//! 1. The user says X.
//! 2. We expect a tool call matching Y.
//! 3. The mock LLM responds with Z.
//! 4. ... repeat ...
//!
//! Scenarios run against the agent runtime. This file implements the
//! declarative builder + assertion logic; the actual agent runtime is wired
//! in by integration tests via the public API.
//!
//! ## Why declarative
//!
//! Hand-rolling the same `assert!(matches!(tool_call.name, "read"))` boilerplate
//! in every test is error-prone. The declarative form puts the assertions next
//! to the expectations and produces much clearer failure messages.

use crate::assertions::{ExpectedText, ExpectedTool};
use crate::mock_llm::{MockLlm, ScriptedResponse};

/// A single step in a scenario script.
#[derive(Debug, Clone)]
pub enum Step {
    /// The user sends this message into the conversation.
    UserSays(String),
    /// Expect the agent to call a tool matching this expectation.
    ExpectToolCall(ExpectedTool),
    /// Mock LLM responds with this scripted response.
    MockResponds(ScriptedResponse),
    /// Expect the agent's text response to satisfy this expectation.
    ExpectText(ExpectedText),
}

/// Builder for an E2E test scenario.
#[derive(Debug, Clone, Default)]
pub struct Scenario {
    pub name: String,
    steps: Vec<Step>,
}

impl Scenario {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), steps: Vec::new() }
    }

    pub fn user_says(mut self, msg: impl Into<String>) -> Self {
        self.steps.push(Step::UserSays(msg.into()));
        self
    }

    pub fn expect_tool_call(mut self, expected: ExpectedTool) -> Self {
        self.steps.push(Step::ExpectToolCall(expected));
        self
    }

    pub fn expect_text(mut self, expected: ExpectedText) -> Self {
        self.steps.push(Step::ExpectText(expected));
        self
    }

    pub fn mock_responds(mut self, resp: ScriptedResponse) -> Self {
        self.steps.push(Step::MockResponds(resp));
        self
    }

    /// Convenience: append `user_says` + `mock_responds` in one call.
    pub fn turn(self, user_msg: impl Into<String>, mock_resp: ScriptedResponse) -> Self {
        self.user_says(user_msg).mock_responds(mock_resp)
    }

    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// Collect the mock LLM script by walking the steps in order and pulling
    /// out each `MockResponds` step.  Useful for wiring the script into a
    /// real agent runtime.
    pub fn into_mock_llm(self) -> (Vec<Step>, MockLlm) {
        let mut mock = MockLlm::new();
        let mut remaining_steps = Vec::new();
        for step in self.steps {
            match step {
                Step::MockResponds(resp) => mock = mock.then(resp),
                other => remaining_steps.push(other),
            }
        }
        (remaining_steps, mock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_builds_steps() {
        let s = Scenario::new("test")
            .user_says("hi")
            .expect_tool_call(ExpectedTool::new("read"))
            .mock_responds(MockLlm::tool_only(ExpectedTool::new("read")))
            .expect_text(ExpectedText::contains("done"));
        assert_eq!(s.steps().len(), 4);
        assert_eq!(s.name, "test");
    }

    #[test]
    fn scenario_turn_helper() {
        let s = Scenario::new("t").turn("hello", MockLlm::text_only(ExpectedText::contains("hi")));
        assert_eq!(s.steps().len(), 2);
    }
}
