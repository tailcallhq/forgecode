//! # forge_e2e
//!
//! End-to-end test harness for the HeliosLite agent.
//!
//! Provides:
//!
//! 1. A **mock LLM provider** that returns canned responses from a script.
//!    No real API keys, no network, deterministic.
//!
//! 2. A **scenario runner** that drives the agent through a multi-turn
//!    conversation with assertions between turns.
//!
//! 3. **Tool-call assertions** so tests can verify the agent called
//!    `read`, `write`, `patch`, etc. with the right arguments.
//!
//! ## Why
//!
//! Without E2E coverage, regressions in the orchestrator loop, tool routing,
//! or context compression slip through to production. Running a real LLM in
//! CI is non-deterministic and expensive; a scripted mock makes the test
//! suite fast and reproducible.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use forge_e2e::{Scenario, MockLlm, ExpectedTool, ExpectedText};
//!
//! // Build a multi-turn conversation script for the mock LLM.
//! let scenario = Scenario::new("agent reads a file then writes a fix")
//!     .user_says("please fix the typo in README.md")
//!     .expect_tool_call(ExpectedTool::new("read").arg("path", "README.md"))
//!     .mock_responds(MockLlm::text_then_tool(
//!         ExpectedText::contains("I'll read it"),
//!         ExpectedTool::new("read").arg("path", "README.md"),
//!     ))
//!     .expect_tool_call(ExpectedTool::new("patch").arg("path", "README.md"))
//!     .mock_responds(MockLlm::tool_only(
//!         ExpectedTool::new("patch").arg("path", "README.md"),
//!     ))
//!     .user_says("thanks")
//!     .mock_responds(MockLlm::text_only(ExpectedText::contains("you're welcome")));
//!
//! // Hand the script to a MockLlm-backed harness and drive the agent.
//! let (steps, mock_llm) = scenario.into_mock_llm();
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! ## Mock LLM Format
//!
//! The mock returns OpenAI-compatible chat completion responses. Tool calls
//! are encoded as `tool_calls` arrays in the response. See [`MockLlm`] for
//! builders.

#![allow(clippy::needless_return)]

mod assertions;
mod mock_llm;
mod scenario;

pub use assertions::{ExpectedText, ExpectedTool};
pub use mock_llm::{MockLlm, ScriptedResponse};
pub use scenario::{Scenario, Step};
