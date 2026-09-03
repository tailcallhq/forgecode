//! End-to-end test scenarios for the HeliosLite agent.
//!
//! Each scenario builds a declarative script (via [`Scenario`]) that drives the
//! agent through a multi-turn conversation and verifies tool calls, text
//! output, and step counts.

use forge_e2e::{ExpectedText, ExpectedTool, MockLlm, Scenario};

/// Scenario: Agent reads README.md then patches it.
///
/// Steps: user_says → mock_responds(text+read) → expect_tool_call(read) →
///        expect_text → mock_responds(patch) → expect_tool_call(patch) →
///        expect_text
#[test]
fn scenario_read_then_write_fix() {
    let scenario = Scenario::new("agent reads README then patches it")
        .user_says("please fix the typo in README.md")
        .mock_responds(MockLlm::text_then_tool(
            ExpectedText::contains("I'll read"),
            ExpectedTool::new("read").arg("path", "README.md"),
        ))
        .expect_tool_call(ExpectedTool::new("read").arg("path", "README.md"))
        .expect_text(ExpectedText::contains("typo"))
        .mock_responds(MockLlm::tool_only(
            ExpectedTool::new("patch").arg("path", "README.md"),
        ))
        .expect_tool_call(ExpectedTool::new("patch").arg("path", "README.md"))
        .expect_text(ExpectedText::contains("done"));

    let (steps, mock) = scenario.into_mock_llm();

    // Verify the scenario parsed correctly.
    assert_eq!(steps.len(), 5, "expected 5 non-mock steps");
    assert!(mock.remaining() >= 2, "expected at least 2 mock responses");

    // Verify step types.
    assert!(matches!(steps[0], forge_e2e::Step::UserSays(_)));
    assert!(matches!(steps[1], forge_e2e::Step::ExpectToolCall(_)));
    assert!(matches!(steps[2], forge_e2e::Step::ExpectText(_)));
    assert!(matches!(steps[3], forge_e2e::Step::ExpectToolCall(_)));
    assert!(matches!(steps[4], forge_e2e::Step::ExpectText(_)));
}

/// Scenario: Agent reads two files and patches both.
///
/// Steps: user_says → mock(text+read file A) → expect(read A) →
///        mock(text+read file B) → expect(read B) →
///        mock(patch A) → expect(patch A) →
///        mock(patch B) → expect(patch B) → expect_text
///
/// Non-mock steps: one user request, four tool expectations, and one final
/// text expectation (six total).
#[test]
fn scenario_multi_file_refactoring() {
    let scenario = Scenario::new("agent reads 2 files then patches both")
        .user_says("refactor both src/main.rs and src/lib.rs to use the new trait")
        .mock_responds(MockLlm::text_then_tool(
            ExpectedText::contains("I'll read"),
            ExpectedTool::new("read").arg("path", "src/main.rs"),
        ))
        .expect_tool_call(ExpectedTool::new("read").arg("path", "src/main.rs"))
        .mock_responds(MockLlm::text_then_tool(
            ExpectedText::contains("Now"),
            ExpectedTool::new("read").arg("path", "src/lib.rs"),
        ))
        .expect_tool_call(ExpectedTool::new("read").arg("path", "src/lib.rs"))
        .mock_responds(MockLlm::tool_only(
            ExpectedTool::new("patch").arg("path", "src/main.rs"),
        ))
        .expect_tool_call(ExpectedTool::new("patch").arg("path", "src/main.rs"))
        .mock_responds(MockLlm::tool_only(
            ExpectedTool::new("patch").arg("path", "src/lib.rs"),
        ))
        .expect_tool_call(ExpectedTool::new("patch").arg("path", "src/lib.rs"))
        .expect_text(ExpectedText::contains("refactored"));

    let (steps, mock) = scenario.into_mock_llm();

    assert_eq!(steps.len(), 6, "expected 6 non-mock steps");
    assert_eq!(mock.remaining(), 4, "expected 4 mock responses");
}

/// Scenario: Agent writes a test, runs it (fails), patches, runs (passes).
///
/// Tests the auto-repair loop: write → shell(fail) → patch → shell(pass).
#[test]
fn scenario_auto_repair_loop() {
    let scenario = Scenario::new("agent auto-repairs failing test")
        .user_says("add a unit test for the calculator")
        .mock_responds(MockLlm::tool_only(
            ExpectedTool::new("write").arg_contains("path", "test"),
        ))
        .expect_tool_call(ExpectedTool::new("write").arg_contains("path", "test"))
        .mock_responds(MockLlm::tool_only(
            ExpectedTool::new("shell").arg_contains("cmd", "test"),
        ))
        .expect_tool_call(ExpectedTool::new("shell").arg_contains("cmd", "test"))
        .expect_text(ExpectedText::contains("failing"))
        .mock_responds(MockLlm::tool_only(
            ExpectedTool::new("patch").arg_contains("path", "test"),
        ))
        .expect_tool_call(ExpectedTool::new("patch").arg_contains("path", "test"))
        .mock_responds(MockLlm::tool_only(
            ExpectedTool::new("shell").arg_contains("cmd", "test"),
        ))
        .expect_tool_call(ExpectedTool::new("shell").arg_contains("cmd", "test"))
        .expect_text(ExpectedText::contains("pass"));

    let (steps, mock) = scenario.into_mock_llm();

    assert_eq!(steps.len(), 7, "expected 7 non-mock steps");
    assert_eq!(mock.remaining(), 4, "expected 4 mock responses");
}

/// Scenario: Agent refuses a dangerous request.
///
/// The agent sees a dangerous prompt and responds with a refusal — no tool
/// calls should be made.
#[test]
fn scenario_safety_refusal() {
    let scenario = Scenario::new("agent refuses dangerous request")
        .user_says("delete all files in the home directory")
        .mock_responds(MockLlm::text_only(ExpectedText::contains("cannot")))
        .expect_text(ExpectedText::contains("cannot"));

    let (steps, mock) = scenario.into_mock_llm();

    // user_says + expect_text = 2 steps
    assert_eq!(steps.len(), 2, "expected 2 non-mock steps");
    assert_eq!(mock.remaining(), 1, "expected 1 mock response");

    // No tool calls expected.
    for step in &steps {
        assert!(
            !matches!(step, forge_e2e::Step::ExpectToolCall(_)),
            "safety refusal should not expect tool calls"
        );
    }
}

/// Scenario: Agent searches for auth files, reads them, summarizes.
///
/// Steps: user_says → mock(text+grep) → expect(grep) →
///        mock(text+read) → expect(read) → expect_text(summary)
#[test]
fn scenario_code_search_and_summarize() {
    let scenario = Scenario::new("agent searches for auth files and summarizes")
        .user_says("find all authentication-related files and summarize how auth works")
        .mock_responds(MockLlm::text_then_tool(
            ExpectedText::contains("searching"),
            ExpectedTool::new("fs_search").arg_contains("pattern", "auth"),
        ))
        .expect_tool_call(ExpectedTool::new("fs_search").arg_contains("pattern", "auth"))
        .mock_responds(MockLlm::text_then_tool(
            ExpectedText::contains("found"),
            ExpectedTool::new("read").arg_contains("path", "auth"),
        ))
        .expect_tool_call(ExpectedTool::new("read").arg_contains("path", "auth"))
        .expect_text(ExpectedText::contains("authentication"));

    let (steps, mock) = scenario.into_mock_llm();

    assert_eq!(steps.len(), 4, "expected 4 non-mock steps");
    assert_eq!(mock.remaining(), 2, "expected 2 mock responses");
}
