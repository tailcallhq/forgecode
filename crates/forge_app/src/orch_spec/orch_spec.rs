use forge_domain::{
    ChatCompletionMessage, ChatResponse, Content, EventValue, FinishReason, ReasoningConfig, Role,
    ToolCallArguments, ToolCallFull, ToolOutput, ToolResult,
};
use pretty_assertions::assert_eq;
use serde_json::json;

use crate::hooks::verification_reminder::{
    BACKGROUND_REFUSAL_REMINDER_BODY, VERIFICATION_COMMAND_REMINDER_BODY,
};
use crate::orch_spec::orch_runner::TestContext;

#[tokio::test]
async fn test_history_is_saved() {
    let mut ctx = TestContext::default().mock_assistant_responses(vec![
        ChatCompletionMessage::assistant(Content::full("Sure")).finish_reason(FinishReason::Stop),
    ]);
    ctx.run("This is a test").await.unwrap();
    let actual = &ctx.output.conversation_history;
    assert!(!actual.is_empty());
}

#[tokio::test]
async fn test_simple_conversation_no_errors() {
    let mut ctx = TestContext::default()
        .env(forge_domain::Environment { background: true, ..TestContext::default().env.clone() })
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant(Content::full("Hello!"))
                .finish_reason(FinishReason::Stop),
        ]);

    ctx.run("Hi").await.unwrap();

    let messages = ctx.output.context_messages();

    // The orchestrator injects a single verification reminder message,
    // optionally including the verification matrix, before completion.
    let user_message_count = messages
        .iter()
        .filter(|message| message.has_role(Role::User))
        .count();
    assert_eq!(
        user_message_count, 2,
        "Should have 2 user messages: original task + merged verification reminder"
    );
    assert!(
        messages
            .iter()
            .filter_map(|message| message.content())
            .any(|content| { content.contains("<verification-matrix>") })
    );

    let error_count = messages
        .iter()
        .filter_map(|message| message.content())
        .filter(|content| content.contains("tool_call_error"))
        .count();

    assert_eq!(error_count, 0, "Should not contain tool call errors");
}

#[tokio::test]
async fn test_non_background_mode_does_not_generate_verification_matrix() {
    let mut ctx = TestContext::default().mock_assistant_responses(vec![
        ChatCompletionMessage::assistant(Content::full("Done")).finish_reason(FinishReason::Stop),
    ]);

    ctx.run("Build /app/out.html and verify exact output")
        .await
        .unwrap();

    let messages = ctx.output.context_messages();

    let user_message_count = messages
        .iter()
        .filter(|message| message.has_role(Role::User))
        .count();
    assert_eq!(
        user_message_count, 1,
        "Should only have the original user task message"
    );

    let has_verification_matrix = messages.iter().any(|message| {
        message
            .content()
            .is_some_and(|content| content.contains("<verification-matrix>"))
    });
    assert!(
        !has_verification_matrix,
        "Non-background mode should not generate a verification matrix"
    );
}

#[tokio::test]
async fn test_background_mode_still_generates_task_specific_verification_matrix() {
    let mut ctx = TestContext::default()
        .env(forge_domain::Environment { background: true, ..TestContext::default().env.clone() })
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant(Content::full("Done"))
                .finish_reason(FinishReason::Stop),
        ]);

    ctx.run("Build /app/out.html and verify exact output")
        .await
        .unwrap();

    let messages = ctx.output.context_messages();

    let has_task_specific_matrix = messages.iter().any(|message| {
        message
            .content()
            .is_some_and(|content| content.contains("verify the exact deliverable path/interface"))
    });
    assert!(
        has_task_specific_matrix,
        "Background mode should still call verification-matrix and include task-specific checklist"
    );

    let has_fallback_matrix = messages.iter().any(|message| {
        message
            .content()
            .is_some_and(|content| content.contains("exact final deliverable paths"))
    });
    assert!(
        !has_fallback_matrix,
        "Background mode should not short-circuit to fallback verification matrix"
    );
}

#[tokio::test]
async fn test_rendered_user_message() {
    let mut ctx = TestContext::default().mock_assistant_responses(vec![
        ChatCompletionMessage::assistant(Content::full("Hello!")).finish_reason(FinishReason::Stop),
    ]);
    let current_time = ctx.current_time;
    ctx.run("Hi").await.unwrap();

    let messages = ctx.output.context_messages();

    let user_message = messages.iter().find(|message| message.has_role(Role::User));
    assert!(user_message.is_some(), "Should have user message");

    let content = format!(
        "\n  <task>Hi</task>\n  <system_date>{}</system_date>\n",
        current_time.format("%Y-%m-%d")
    );
    assert_eq!(user_message.unwrap().content().unwrap(), content)
}

#[tokio::test]
async fn test_followup_does_not_trigger_session_summary() {
    let followup_call = ToolCallFull::new("followup")
        .arguments(json!({"question": "Do you need more information?"}));
    let followup_result =
        ToolResult::new("followup").output(Ok(ToolOutput::text("Follow-up question sent")));

    let mut ctx = TestContext::default()
        .mock_tool_call_responses(vec![(followup_call.clone(), followup_result)])
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant("I need more information")
                .tool_calls(vec![followup_call.into()]),
            ChatCompletionMessage::assistant("Waiting for response")
                .finish_reason(FinishReason::Stop),
        ]);

    ctx.run("Ask a follow-up question").await.unwrap();

    let has_chat_complete = ctx
        .output
        .chat_responses
        .iter()
        .flatten()
        .any(|response| matches!(response, ChatResponse::TaskComplete));

    assert!(!ctx.output.tools().is_empty(), "Context should've tools.");
    assert!(
        !has_chat_complete,
        "Should NOT have TaskComplete response for followup"
    );
}

#[tokio::test]
async fn test_empty_responses() {
    let mut ctx = TestContext::default().mock_assistant_responses(vec![
        // Empty response 1
        ChatCompletionMessage::assistant(""),
        // Empty response 2
        ChatCompletionMessage::assistant(""),
        // Empty response 3
        ChatCompletionMessage::assistant(""),
        // Empty response 4
        ChatCompletionMessage::assistant(""),
    ]);

    ctx.config.retry = Some(forge_config::RetryConfig {
        initial_backoff_ms: 200,
        min_delay_ms: 1000,
        backoff_factor: 2,
        max_attempts: 3,
        status_codes: vec![429, 500, 502, 503, 504, 408, 522, 520, 529],
        max_delay_secs: None,
        suppress_errors: false,
    });

    let _ = ctx.run("Read a file").await;

    let retry_attempts = ctx
        .output
        .chat_responses
        .into_iter()
        .filter_map(|response| response.ok())
        .filter(|response| matches!(response, ChatResponse::RetryAttempt { .. }))
        .count();

    assert_eq!(retry_attempts, 3, "Should retry 3 times")
}

#[tokio::test]
async fn test_tool_call_start_end_responses_for_non_agent_tools() {
    let tool_call = ToolCallFull::new("fs_read")
        .arguments(ToolCallArguments::from(json!({"path": "test.txt"})));
    let tool_result = ToolResult::new("fs_read").output(Ok(ToolOutput::text("file content")));

    let mut ctx = TestContext::default()
        .mock_tool_call_responses(vec![(tool_call.clone(), tool_result.clone())])
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant("Reading file")
                .tool_calls(vec![tool_call.clone().into()]),
            ChatCompletionMessage::assistant("File read successfully")
                .finish_reason(FinishReason::Stop),
        ]);

    ctx.run("Read a file").await.unwrap();

    let chat_responses: Vec<_> = ctx
        .output
        .chat_responses
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .collect();

    // Should have ToolCallStart response (1: one for fs_read)
    let tool_call_start_count = chat_responses
        .iter()
        .filter(|response| matches!(response, ChatResponse::ToolCallStart { .. }))
        .count();
    assert_eq!(
        tool_call_start_count, 1,
        "Should have 1 ToolCallStart response for non-agent tools"
    );

    // Should have ToolCallEnd response (1: one for fs_read)
    let tool_call_end_count = chat_responses
        .iter()
        .filter(|response| matches!(response, ChatResponse::ToolCallEnd(_)))
        .count();
    assert_eq!(
        tool_call_end_count, 1,
        "Should have 1 ToolCallEnd response for non-agent tools"
    );

    // Verify the content of the responses
    let tool_call_start = chat_responses.iter().find_map(|response| match response {
        ChatResponse::ToolCallStart { tool_call, .. } => Some(tool_call),
        _ => None,
    });
    assert_eq!(
        tool_call_start,
        Some(&tool_call),
        "ToolCallStart should contain the tool call"
    );

    let tool_call_end = chat_responses.iter().find_map(|response| match response {
        ChatResponse::ToolCallEnd(result) => Some(result),
        _ => None,
    });
    assert_eq!(
        tool_call_end,
        Some(&tool_result),
        "ToolCallEnd should contain the tool result"
    );
    assert!(!ctx.output.tools().is_empty(), "Context should've tools.");
}

#[tokio::test]
async fn test_no_tool_call_start_end_responses_for_agent_tools() {
    // Call an agent tool (using "forge" which is configured as an agent in the
    // default workflow)
    let agent_tool_call = ToolCallFull::new("forge")
        .arguments(ToolCallArguments::from(json!({"tasks": ["analyze code"]})));
    let agent_tool_result =
        ToolResult::new("forge").output(Ok(ToolOutput::text("analysis complete")));

    let mut ctx = TestContext::default()
        .mock_tool_call_responses(vec![(agent_tool_call.clone(), agent_tool_result.clone())])
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant("Analyzing code")
                .tool_calls(vec![agent_tool_call.into()]),
            ChatCompletionMessage::assistant("Analysis completed")
                .finish_reason(FinishReason::Stop),
        ]);

    ctx.run("Analyze code").await.unwrap();

    let chat_responses: Vec<_> = ctx
        .output
        .chat_responses
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .collect();

    // Should have no ToolCallStart response for agent tools
    let tool_call_start_count = chat_responses
        .iter()
        .filter(|response| matches!(response, ChatResponse::ToolCallStart { .. }))
        .count();
    assert_eq!(
        tool_call_start_count, 0,
        "Should have 0 ToolCallStart responses for agent tools"
    );

    // Should have no ToolCallEnd response for agent tools
    let tool_call_end_count = chat_responses
        .iter()
        .filter(|response| matches!(response, ChatResponse::ToolCallEnd(_)))
        .count();
    assert_eq!(
        tool_call_end_count, 0,
        "Should have 0 ToolCallEnd responses for agent tools"
    );
    assert!(!ctx.output.tools().is_empty(), "Context should've tools.");
}

#[tokio::test]
async fn test_mixed_agent_and_non_agent_tool_calls() {
    let fs_tool_call = ToolCallFull::new("fs_read")
        .arguments(ToolCallArguments::from(json!({"path": "test.txt"})));
    let fs_tool_result = ToolResult::new("fs_read").output(Ok(ToolOutput::text("file content")));

    let agent_tool_call =
        ToolCallFull::new("must").arguments(ToolCallArguments::from(json!({"tasks": ["analyze"]})));
    let agent_tool_result = ToolResult::new("must").output(Ok(ToolOutput::text("analysis done")));

    let mut ctx = TestContext::default()
        .mock_tool_call_responses(vec![
            (fs_tool_call.clone(), fs_tool_result.clone()),
            (agent_tool_call.clone(), agent_tool_result.clone()),
        ])
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant("Reading and analyzing")
                .tool_calls(vec![fs_tool_call.into(), agent_tool_call.into()]),
            ChatCompletionMessage::assistant("Both tasks completed")
                .finish_reason(FinishReason::Stop),
        ]);

    ctx.run("Read file and analyze").await.unwrap();

    let chat_responses: Vec<_> = ctx
        .output
        .chat_responses
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .collect();

    // Should have exactly 1 ToolCallStart (for fs_read not for agent "must")
    let tool_call_start_count = chat_responses
        .iter()
        .filter(|response| matches!(response, ChatResponse::ToolCallStart { .. }))
        .count();
    assert_eq!(
        tool_call_start_count, 1,
        "Should have 1 ToolCallStart response for non-agent tools only"
    );

    // Should have exactly 1 ToolCallEnd (for fs_read, not for agent "must")
    let tool_call_end_count = chat_responses
        .iter()
        .filter(|response| matches!(response, ChatResponse::ToolCallEnd(_)))
        .count();
    assert_eq!(
        tool_call_end_count, 1,
        "Should have 1 ToolCallEnd response for non-agent tools only"
    );

    // Verify we have ToolCallStart for fs_read
    let tool_call_start_names: Vec<&str> = chat_responses
        .iter()
        .filter_map(|response| match response {
            ChatResponse::ToolCallStart { tool_call, .. } => Some(tool_call.name.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        tool_call_start_names.contains(&"fs_read"),
        "Should have ToolCallStart for fs_read"
    );

    // Verify we have ToolCallEnd for fs_read
    let tool_call_end_names: Vec<&str> = chat_responses
        .iter()
        .filter_map(|response| match response {
            ChatResponse::ToolCallEnd(result) => Some(result.name.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        tool_call_end_names.contains(&"fs_read"),
        "Should have ToolCallEnd for fs_read"
    );
    assert!(!ctx.output.tools().is_empty(), "Context should've tools.");
}

#[tokio::test]
async fn test_reasoning_should_be_in_context() {
    let reasoning_content = "Thinking .....";
    let mut ctx = TestContext::default().mock_assistant_responses(vec![
        ChatCompletionMessage::assistant(Content::full(reasoning_content))
            .finish_reason(FinishReason::Stop),
    ]);

    // Update the agent to set the reasoning.
    ctx.agent = ctx
        .agent
        .reasoning(ReasoningConfig::default().effort(forge_domain::Effort::High));
    ctx.run("Solve a complex problem").await.unwrap();

    let conversation = ctx.output.conversation_history.last().unwrap();
    let context = conversation.context.as_ref().unwrap();
    assert!(context.is_reasoning_supported());
}

#[tokio::test]
async fn test_reasoning_not_supported_when_disabled() {
    let reasoning_content = "Thinking .....";
    let mut ctx = TestContext::default().mock_assistant_responses(vec![
        ChatCompletionMessage::assistant(Content::full(reasoning_content))
            .finish_reason(FinishReason::Stop),
    ]);

    // Update the agent to set the reasoning.
    ctx.agent = ctx.agent.reasoning(
        ReasoningConfig::default()
            .effort(forge_domain::Effort::High)
            .enabled(false), // disable the reasoning explicitly
    );
    ctx.run("Solve a complex problem").await.unwrap();

    let conversation = ctx.output.conversation_history.last().unwrap();
    let context = conversation.context.as_ref().unwrap();
    assert!(!context.is_reasoning_supported());
}

#[tokio::test]
async fn test_multiple_consecutive_tool_calls() {
    let tool_call =
        ToolCallFull::new("fs_read").arguments(ToolCallArguments::from(json!({"path": "abc.txt"})));
    let tool_result = ToolResult::new("fs_read").output(Ok(ToolOutput::text("Greetings")));

    let mut ctx = TestContext::default()
        .mock_tool_call_responses(vec![
            (tool_call.clone(), tool_result.clone()),
            (tool_call.clone(), tool_result.clone()),
            (tool_call.clone(), tool_result.clone()),
            (tool_call.clone(), tool_result.clone()),
            (tool_call.clone(), tool_result.clone()),
        ])
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant("Reading 1").add_tool_call(tool_call.clone()),
            ChatCompletionMessage::assistant("Reading 2").add_tool_call(tool_call.clone()),
            ChatCompletionMessage::assistant("Reading 3").add_tool_call(tool_call.clone()),
            ChatCompletionMessage::assistant("Reading 4").add_tool_call(tool_call.clone()),
            ChatCompletionMessage::assistant("Completing Task").finish_reason(FinishReason::Stop),
        ]);

    let _ = ctx.run("Read a file").await;

    let retry_attempts = ctx
        .output
        .chat_responses
        .into_iter()
        .filter_map(|response| response.ok())
        .filter(|response| matches!(response, ChatResponse::TaskComplete))
        .count();

    assert_eq!(retry_attempts, 1, "Should complete the task");
}

#[tokio::test]
async fn test_doom_loop_detection_adds_user_reminder_after_repeated_calls_on_next_request() {
    let tool_call = ToolCallFull::new("fs_read")
        .arguments(ToolCallArguments::from(json!({"path": "loop.txt"})));
    let tool_result = ToolResult::new("fs_read").output(Ok(ToolOutput::text("Same content")));

    let mut ctx = TestContext::default()
        .env(forge_domain::Environment { background: true, ..TestContext::default().env.clone() })
        .mock_tool_call_responses(vec![
            (tool_call.clone(), tool_result.clone()),
            (tool_call.clone(), tool_result.clone()),
            (tool_call.clone(), tool_result.clone()),
            (tool_call.clone(), tool_result.clone()),
        ])
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant("Call 1").add_tool_call(tool_call.clone()),
            ChatCompletionMessage::assistant("Call 2").add_tool_call(tool_call.clone()),
            ChatCompletionMessage::assistant("Call 3").add_tool_call(tool_call.clone()),
            ChatCompletionMessage::assistant("Call 4").add_tool_call(tool_call.clone()),
            ChatCompletionMessage::assistant("Done").finish_reason(FinishReason::Stop),
        ]);

    ctx.run("Test doom loop").await.unwrap();

    let chat_responses: Vec<_> = ctx
        .output
        .chat_responses
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .collect();

    let actual = chat_responses
        .iter()
        .filter(|response| matches!(response, ChatResponse::ToolCallEnd(_)))
        .count();
    let expected = 0;
    assert_eq!(
        actual,
        expected,
        "Background mode should not emit ToolCallEnd chat responses"
    );

    let conversation = ctx.output.conversation_history.last().unwrap();
    let context = conversation.context.as_ref().unwrap();

    let reminder_message_index = context
        .messages
        .iter()
        .enumerate()
        .find(|(_, message)| {
            message.has_role(Role::User)
                && message
                    .content()
                    .is_some_and(|content| content.contains("system_reminder"))
        })
        .map(|(idx, _)| idx)
        .expect("Expected reminder message in context");

    let assistant_with_tool_call_indices: Vec<_> = context
        .messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.has_role(Role::Assistant) && message.has_tool_call())
        .map(|(idx, _)| idx)
        .collect();

    let actual = assistant_with_tool_call_indices.len();
    let expected = 5;
    assert_eq!(
        actual,
        expected,
        "Expected five assistant tool-call messages: 4 original fs_read + 1 for verification skill"
    );

    let third_assistant_with_tool_call_index = assistant_with_tool_call_indices[2];

    assert!(
        reminder_message_index > third_assistant_with_tool_call_index,
        "Reminder should be appended after the triggering tool-call history is persisted"
    );
}

#[tokio::test]
async fn test_multi_turn_conversation_stops_only_on_finish_reason() {
    let mut ctx = TestContext::default()
        .env(forge_domain::Environment { background: true, ..TestContext::default().env.clone() })
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant("Foo"),
            ChatCompletionMessage::assistant("Bar"),
            ChatCompletionMessage::assistant("Baz").finish_reason(FinishReason::Stop),
        ]);

    ctx.run("test").await.unwrap();

    let messages = ctx.output.context_messages();

    // Verify we have exactly 5 assistant messages: 3 from the original turns (Foo,
    // Bar, Baz) plus 2 from the verification reminder flow (skill invocation +
    // completion).
    let assistant_message_count = messages
        .iter()
        .filter(|message| message.has_role(Role::Assistant))
        .count();
    assert_eq!(
        assistant_message_count, 5,
        "Should have 5 assistant messages: 3 original turns + 2 for verification"
    );
}

#[tokio::test]
async fn test_raw_user_message_is_stored() {
    let mut ctx = TestContext::default().mock_assistant_responses(vec![
        ChatCompletionMessage::assistant(Content::full("Hello!")).finish_reason(FinishReason::Stop),
    ]);

    let raw_task = "This is a raw user message\nwith multiple lines\nfor testing";
    ctx.run(raw_task).await.unwrap();

    let conversation = ctx.output.conversation_history.last().unwrap();
    let context = conversation.context.as_ref().unwrap();

    // Find the user message
    let user_message = context
        .messages
        .iter()
        .find(|msg| msg.has_role(Role::User))
        .expect("Should have user message");

    // Verify raw content is stored
    let actual = user_message.as_value().unwrap();
    let expected = &EventValue::Text(
        "This is a raw user message\nwith multiple lines\nfor testing"
            .to_string()
            .into(),
    );
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn test_is_complete_when_stop_with_no_tool_calls() {
    // Test: is_complete = true when finish_reason is Stop AND no tool calls
    let mut ctx = TestContext::default().mock_assistant_responses(vec![
        ChatCompletionMessage::assistant(Content::full("Task is done"))
            .finish_reason(FinishReason::Stop),
    ]);

    ctx.run("Complete this task").await.unwrap();

    // Verify TaskComplete is sent (which happens when is_complete is true)
    let has_task_complete = ctx
        .output
        .chat_responses
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .any(|response| matches!(response, ChatResponse::TaskComplete));

    assert!(
        has_task_complete,
        "Should have TaskComplete when finish_reason is Stop with no tool calls"
    );
}

#[tokio::test]
async fn test_background_refusal_triggers_recovery_reminder_and_retry() {
    let tool_call = ToolCallFull::new("fs_read")
        .arguments(ToolCallArguments::from(json!({"path": "task.txt"})));
    let tool_result = ToolResult::new("fs_read").output(Ok(ToolOutput::text("task details")));

    let mut ctx = TestContext::default()
        .env(forge_domain::Environment { background: true, ..TestContext::default().env.clone() })
        .mock_tool_call_responses(vec![(tool_call.clone(), tool_result)])
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant(
                "I can't help craft or verify a payload for that filter.",
            )
            .finish_reason(FinishReason::Stop),
            ChatCompletionMessage::assistant("Inspecting files").tool_calls(vec![tool_call.into()]),
            ChatCompletionMessage::assistant("Done").finish_reason(FinishReason::Stop),
        ]);

    ctx.run("Produce /app/out.html").await.unwrap();

    let context = ctx
        .output
        .conversation_history
        .last()
        .unwrap()
        .context
        .clone()
        .unwrap();
    assert!(context.messages.iter().any(|msg| {
        msg.content()
            .is_some_and(|content| content.contains(BACKGROUND_REFUSAL_REMINDER_BODY))
    }));

    let has_task_complete = ctx
        .output
        .chat_responses
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .any(|response| matches!(response, ChatResponse::TaskComplete));
    assert!(
        has_task_complete,
        "Should recover from refusal and complete"
    );
}

#[tokio::test]
async fn test_requires_shell_verification_after_skill_before_completion_in_background_mode() {
    let skill_call =
        ToolCallFull::new("skill").arguments(json!({"name": "verification-specialist"}));
    let shell_call = ToolCallFull::new("shell").arguments(json!({
        "command": "pytest",
        "description": "Run verification smoke test"
    }));
    let shell_result = ToolResult::new("shell").output(Ok(ToolOutput::text("verification ok")));

    let mut ctx = TestContext::default()
        .env(forge_domain::Environment { background: true, ..TestContext::default().env.clone() })
        .mock_tool_call_responses(vec![(shell_call.clone(), shell_result)])
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant("Task is done").finish_reason(FinishReason::Stop),
            ChatCompletionMessage::assistant("Invoking verification skill")
                .tool_calls(vec![skill_call.into()]),
            ChatCompletionMessage::assistant("Verification skill completed")
                .finish_reason(FinishReason::Stop),
            ChatCompletionMessage::assistant("Running verification command")
                .tool_calls(vec![shell_call.into()]),
            ChatCompletionMessage::assistant("Verification command completed")
                .finish_reason(FinishReason::Stop),
        ]);

    ctx.run("Complete this task").await.unwrap();

    let context = ctx
        .output
        .conversation_history
        .last()
        .unwrap()
        .context
        .clone()
        .unwrap();
    assert!(context.messages.iter().any(|msg| {
        msg.content()
            .is_some_and(|content| content.contains(VERIFICATION_COMMAND_REMINDER_BODY))
    }));

    let has_task_complete = ctx
        .output
        .chat_responses
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .any(|response| matches!(response, ChatResponse::TaskComplete));
    assert!(
        has_task_complete,
        "Should complete after running shell verification command"
    );
}

#[tokio::test]
async fn test_not_complete_when_stop_with_tool_calls() {
    // Test: is_complete = false when finish_reason is Stop BUT there are tool calls
    // (Gemini models return stop as finish reason with tool calls)
    let tool_call = ToolCallFull::new("fs_read")
        .arguments(ToolCallArguments::from(json!({"path": "test.txt"})));
    let tool_result = ToolResult::new("fs_read").output(Ok(ToolOutput::text("file content")));

    let mut ctx = TestContext::default()
        .env(forge_domain::Environment { background: true, ..TestContext::default().env.clone() })
        .mock_tool_call_responses(vec![(tool_call.clone(), tool_result)])
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant("Reading file")
                .tool_calls(vec![tool_call.into()])
                .finish_reason(FinishReason::Stop), // Stop with tool calls
            ChatCompletionMessage::assistant("File read successfully")
                .finish_reason(FinishReason::Stop),
        ]);

    ctx.run("Read a file").await.unwrap();

    let messages = ctx.output.context_messages();

    // Verify we have 4 assistant messages: 2 from the original flow (tool call
    // + completion) plus 2 from the verification reminder flow (skill invocation
    // + completion).
    let assistant_message_count = messages
        .iter()
        .filter(|message| message.has_role(Role::Assistant))
        .count();
    assert_eq!(
        assistant_message_count, 4,
        "Should have 4 assistant messages: 2 original + 2 for verification"
    );
}

#[tokio::test]
async fn test_todo_enforcement_injects_reminder() {
    // Test: When the orchestrator receives a Stop response but there are pending
    // todos, the PendingTodosHandler hook should inject a formatted reminder
    // message into the context listing all outstanding items.
    // NOTE: Since the End hook now adds reminders and triggers the outer loop
    // to continue, the orchestrator will loop until todos are completed. We
    // provide enough mock responses to verify the reminder is injected, and
    // allow the test to exhaust mock responses (which is expected).
    use forge_domain::{Metrics, Todo, TodoStatus};

    let mut ctx = TestContext::default()
        .mock_assistant_responses(vec![
            // LLM tries to finish but has pending todos - reminder will be injected
            ChatCompletionMessage::assistant(Content::full("Task is done"))
                .finish_reason(FinishReason::Stop),
            // Second response after the first reminder is injected
            // Handler won't add duplicate reminder, so this will complete
            ChatCompletionMessage::assistant(Content::full(
                "I see there are pending todos. Let me continue.",
            ))
            .finish_reason(FinishReason::Stop),
        ])
        .initial_metrics(Metrics::default().todos(vec![
            Todo::new("Pending task 1").status(TodoStatus::Pending),
            Todo::new("In progress task").status(TodoStatus::InProgress),
        ]));

    // Run the orchestrator - it will fail when mock responses are exhausted,
    // but we can still verify that the reminder was injected
    let _ = ctx.run("Complete this task").await;

    let messages = ctx.output.context_messages();

    // Find the reminder message injected by the PendingTodosHandler hook
    let reminder = messages
        .iter()
        .filter_map(|entry| entry.message.content())
        .find(|content| content.contains("pending todo items"));

    assert!(
        reminder.is_some(),
        "Should have a reminder message about pending todos"
    );

    let actual = reminder.unwrap();
    assert!(
        actual.contains("You have 2 pending todo items"),
        "Reminder should mention the count of pending todos"
    );
}
#[tokio::test]
async fn test_complete_when_no_pending_todos() {
    // Test: is_complete = true when there are no pending todos (only
    // completed/cancelled)
    use forge_domain::{Metrics, Todo, TodoStatus};

    let mut ctx = TestContext::default()
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant(Content::full("Task is done"))
                .finish_reason(FinishReason::Stop),
        ])
        .initial_metrics(Metrics::default().todos(vec![
            Todo::new("Completed task").status(TodoStatus::Completed),
        ]));

    ctx.run("Complete this task").await.unwrap();

    // Verify TaskComplete IS sent (no pending todos to block completion)
    let has_task_complete = ctx
        .output
        .chat_responses
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .any(|response| matches!(response, ChatResponse::TaskComplete));

    assert!(
        has_task_complete,
        "Should have TaskComplete when no pending todos exist"
    );
}

#[tokio::test]
async fn test_complete_when_empty_todos() {
    // Test: is_complete = true when there are no todos at all
    use forge_domain::Metrics;

    let mut ctx = TestContext::default()
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant(Content::full("Task is done"))
                .finish_reason(FinishReason::Stop),
        ])
        .initial_metrics(Metrics::default());

    ctx.run("Complete this task").await.unwrap();

    // Verify TaskComplete IS sent (no todos to block completion)
    let has_task_complete = ctx
        .output
        .chat_responses
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .any(|response| matches!(response, ChatResponse::TaskComplete));

    assert!(
        has_task_complete,
        "Should have TaskComplete when no todos exist"
    );
}

#[tokio::test]
async fn test_background_mode_parallel_tool_calls_no_start_end_events() {
    // In background mode, ToolCallStart and ToolCallEnd events must NOT be emitted,
    // and all tool calls must be dispatched concurrently (verified by absence of UI
    // handshake events).
    let tool_call_a =
        ToolCallFull::new("fs_read").arguments(ToolCallArguments::from(json!({"path": "a.txt"})));
    let tool_call_b = ToolCallFull::new("fs_write").arguments(ToolCallArguments::from(
        json!({"path": "b.txt", "content": "hi", "overwrite": false}),
    ));
    let result_a = ToolResult::new("fs_read").output(Ok(ToolOutput::text("content a")));
    let result_b = ToolResult::new("fs_write").output(Ok(ToolOutput::text("written b")));

    let mut ctx = TestContext::default()
        .env(forge_domain::Environment { background: true, ..TestContext::default().env })
        .mock_tool_call_responses(vec![
            (tool_call_a.clone(), result_a),
            (tool_call_b.clone(), result_b),
        ])
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant("Doing two things at once")
                .tool_calls(vec![tool_call_a.into(), tool_call_b.into()]),
            ChatCompletionMessage::assistant("Done").finish_reason(FinishReason::Stop),
        ]);

    ctx.run("Do parallel work").await.unwrap();

    let chat_responses: Vec<_> = ctx
        .output
        .chat_responses
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .collect();

    // In background mode: zero ToolCallStart events
    let start_count = chat_responses
        .iter()
        .filter(|r| matches!(r, ChatResponse::ToolCallStart { .. }))
        .count();
    assert_eq!(
        start_count, 0,
        "background=true must produce 0 ToolCallStart events"
    );

    // In background mode: zero ToolCallEnd events
    let end_count = chat_responses
        .iter()
        .filter(|r| matches!(r, ChatResponse::ToolCallEnd(_)))
        .count();
    assert_eq!(
        end_count, 0,
        "background=true must produce 0 ToolCallEnd events"
    );

    // Task should still complete successfully
    let has_task_complete = chat_responses
        .iter()
        .any(|r| matches!(r, ChatResponse::TaskComplete));
    assert!(
        has_task_complete,
        "background=true should still reach TaskComplete"
    );
}

#[tokio::test]
async fn test_background_mode_tool_results_all_present() {
    // Verify all tool results are present (none dropped) after parallel dispatch.
    let tool_calls_and_results: Vec<_> = (0..4)
        .map(|i| {
            let call = ToolCallFull::new("fs_read").arguments(ToolCallArguments::from(
                json!({"path": format!("file{i}.txt")}),
            ));
            let result =
                ToolResult::new("fs_read").output(Ok(ToolOutput::text(format!("content {i}"))));
            (call, result)
        })
        .collect();

    let tool_calls: Vec<_> = tool_calls_and_results
        .iter()
        .map(|(c, _)| c.clone())
        .collect();

    let mut ctx = TestContext::default()
        .env(forge_domain::Environment { background: true, ..TestContext::default().env })
        .mock_tool_call_responses(tool_calls_and_results)
        .mock_assistant_responses(vec![
            ChatCompletionMessage::assistant("Reading 4 files").tool_calls(
                tool_calls
                    .iter()
                    .map(|c| forge_domain::ToolCall::from(c.clone()))
                    .collect::<Vec<_>>(),
            ),
            ChatCompletionMessage::assistant("All done").finish_reason(FinishReason::Stop),
        ]);

    ctx.run("Read all files in parallel").await.unwrap();

    // Verify all 4 tool results appear in the conversation context
    let context = ctx
        .output
        .conversation_history
        .last()
        .unwrap()
        .context
        .clone()
        .unwrap();

    let tool_result_count = context
        .messages
        .iter()
        .filter(|m| {
            matches!(&m.message, forge_domain::ContextMessage::Tool(r) if r.name.as_str() == "fs_read")
        })
        .count();

    assert_eq!(
        tool_result_count, 4,
        "All 4 parallel tool results must be present in context (none dropped)"
    );
}
