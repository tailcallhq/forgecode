//! End-to-end agent-loop tests (F-E9) using a scripted mock LLM provider.
//!
//! These tests drive the *full* orchestrator agent loop — user message →
//! assistant tool_call → tool result → final answer — through the same
//! [`Runner`] / [`TestContext`] harness used by the rest of the orchestration
//! spec suite. The mock provider returns scripted chat-completion messages and
//! scripted tool results, so no real LLM API or network is ever hit.
//!
//! The validations are deliberately best-effort and pragmatic: they assert the
//! final assistant response text, that tool interactions happened in the
//! expected order, and that the produced conversation records survive a
//! serialization round-trip (the persistence boundary used by the real repo).

use forge_domain::{
    ChatCompletionMessage, ChatResponse, Content, FinishReason, Role, ToolCallArguments,
    ToolCallFull, ToolOutput, ToolResult,
};
use serde_json::json;

use crate::orch_spec::orch_runner::TestContext;

/// The complete happy-path agent loop, exercised end to end:
///   1. user message drives the orchestrator
///   2. the (mocked) LLM emits an assistant message with a `fs_read` tool call
///   3. the (mocked) tool result is fed back into the context
///   4. the LLM emits a final answer with `FinishReason::Stop`
///
/// We assert the final assistant text, that the tool interaction happened in
/// the expected order (ToolCallStart before ToolCallEnd), and that the
/// produced conversation + context serialize cleanly so it can be persisted.
#[tokio::test]
async fn full_agent_loop_user_message_tool_call_tool_result_final_answer() {
    let tool_call =
        ToolCallFull::new("fs_read").arguments(ToolCallArguments::from(json!({"path": "a.txt"})));
    let tool_result = ToolResult::new("fs_read").output(Ok(ToolOutput::text("contents of a.txt")));

    let mut ctx = TestContext::default()
        .mock_tool_call_responses(vec![(tool_call.clone(), tool_result.clone())])
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant("I will read the file")
                .tool_calls(vec![tool_call.clone().into()]),
            ChatCompletionMessage::assistant(Content::full(
                "Final answer: the file contains 'contents of a.txt'",
            ))
            .finish_reason(FinishReason::Stop),
        ]);

    ctx.run("Read a.txt for me").await.unwrap();

    // ---- The final assistant answer is present and correct -----------------
    let messages = ctx.output.context_messages();
    let assistant_contents: Vec<&str> = messages
        .iter()
        .filter(|m| m.has_role(Role::Assistant))
        .filter_map(|m| m.content())
        .collect();
    assert_eq!(
        assistant_contents,
        vec![
            "I will read the file",
            "Final answer: the file contains 'contents of a.txt'",
        ],
        "Mismatched assistant messages across the agent loop"
    );
    assert!(
        assistant_contents
            .last()
            .unwrap()
            .contains("contents of a.txt"),
        "Final assistant answer should reference the tool result"
    );

    // ---- Tool interactions happened in the expected order ------------------
    let chat_responses: Vec<_> = ctx
        .output
        .chat_responses
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .collect();

    let tool_call_start = chat_responses.iter().find_map(|r| match r {
        ChatResponse::ToolCallStart { tool_call, .. } => Some(tool_call),
        _ => None,
    });
    assert_eq!(
        tool_call_start,
        Some(&tool_call),
        "ToolCallStart must carry the fs_read tool call"
    );

    let tool_call_end = chat_responses.iter().find_map(|r| match r {
        ChatResponse::ToolCallEnd(result) => Some(result),
        _ => None,
    });
    assert_eq!(
        tool_call_end,
        Some(&tool_result),
        "ToolCallEnd must carry the fs_read tool result"
    );

    let start_idx = chat_responses
        .iter()
        .position(|r| matches!(r, ChatResponse::ToolCallStart { .. }));
    let end_idx = chat_responses
        .iter()
        .position(|r| matches!(r, ChatResponse::ToolCallEnd(_)));
    assert!(
        start_idx.is_some() && end_idx.is_some() && start_idx < end_idx,
        "ToolCallStart must precede ToolCallEnd"
    );

    // The task must have been declared complete.
    let has_task_complete = chat_responses
        .iter()
        .any(|r| matches!(r, ChatResponse::TaskComplete));
    assert!(
        has_task_complete,
        "Expected TaskComplete at end of agent loop"
    );

    // ---- Conversation records persist across the write→read boundary -------
    let conversation = ctx
        .output
        .conversation_history
        .last()
        .expect("conversation must be recorded in history");
    let context = conversation
        .context
        .as_ref()
        .expect("persisted conversation must carry context");

    // The context contains the user task, the assistant tool call, the tool
    // result, and the final assistant answer in order.
    let role_sequence: Vec<&str> = context
        .messages
        .iter()
        .map(|m| {
            if m.has_role(Role::Assistant) && m.has_tool_call() {
                "assistant(tool_call)"
            } else if m.has_role(Role::Assistant) {
                "assistant"
            } else if m.has_tool_result() {
                "tool"
            } else if m.has_role(Role::User) {
                "user"
            } else {
                "other"
            }
        })
        .collect();

    assert!(
        role_sequence.contains(&"user"),
        "context should contain the user task message"
    );
    assert!(
        role_sequence.contains(&"assistant(tool_call)"),
        "context should contain an assistant tool-call message"
    );
    assert!(
        role_sequence.contains(&"tool"),
        "context should contain the tool result message"
    );

    let assistant_tool_idx = context
        .messages
        .iter()
        .position(|m| m.has_role(Role::Assistant) && m.has_tool_call())
        .unwrap();
    let tool_result_idx = context
        .messages
        .iter()
        .position(|m| m.has_tool_result())
        .unwrap();
    assert!(
        assistant_tool_idx < tool_result_idx,
        "assistant tool call must precede the tool result message"
    );

    // Serialization round-trip proves the record can cross the repo write/read
    // boundary (the real repository serializes/deserializes Conversations).
    let serialized = serde_json::to_string(conversation).expect("conversation must serialize");
    let deserialized: forge_domain::Conversation =
        serde_json::from_str(&serialized).expect("conversation must deserialize");
    let deserialized_contents: Vec<String> = deserialized
        .context
        .as_ref()
        .map(|c| {
            c.messages
                .iter()
                .filter_map(|m| m.content().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        deserialized_contents
            .last()
            .is_some_and(|c| c.contains("Final answer")),
        "deserialized conversation should retain the final assistant answer"
    );
}

/// A minimal loop where the LLM emits only a tool call first, then, after the
/// mock tool result, a stop-with-no-tool-calls answer. Guards that the
/// orchestrator does not terminate before feeding the tool result back.
#[tokio::test]
async fn tool_call_then_final_answer_does_not_terminate_early() {
    let tool_call = ToolCallFull::new("fs_read")
        .arguments(ToolCallArguments::from(json!({"path": "loop.txt"})));
    let tool_result = ToolResult::new("fs_read").output(Ok(ToolOutput::text("once")));

    let mut ctx = TestContext::default()
        .mock_tool_call_responses(vec![(tool_call.clone(), tool_result.clone())])
        .mock_assistant_responses(vec![
            // Largest finish reason is Stop but WITH a tool call → must not
            // terminate (Gemini-style behavior).
            ChatCompletionMessage::assistant("call")
                .tool_calls(vec![tool_call.clone().into()])
                .finish_reason(FinishReason::Stop),
            ChatCompletionMessage::assistant(Content::full("done"))
                .finish_reason(FinishReason::Stop),
        ]);

    ctx.run("loop once").await.unwrap();

    let messages = ctx.output.context_messages();
    let tool_results = messages.iter().filter(|m| m.has_tool_result()).count();
    assert_eq!(tool_results, 1, "expected exactly one tool result fed back");

    let assistant_with_tool = messages
        .iter()
        .filter(|m| m.has_role(Role::Assistant) && m.has_tool_call())
        .count();
    assert_eq!(
        assistant_with_tool, 1,
        "expected exactly one assistant tool-call message"
    );

    let final_answers: Vec<&str> = messages
        .iter()
        .filter(|m| m.has_role(Role::Assistant) && !m.has_tool_call())
        .filter_map(|m| m.content())
        .collect();
    assert!(
        final_answers.last().is_some_and(|c| *c == "done"),
        "final answer after tool result should be 'done'"
    );
}
