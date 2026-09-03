//! End-to-end tests for the agent loop with mock LLM providers.
//!
//! These tests exercise the core interaction patterns that `forge_api`
//! orchestrates — prompt -> tool_call -> tool execution -> final answer —
//! without any network calls or infrastructure dependencies.

use std::sync::Arc;
use std::time::{Duration, Instant};

use forge_api::*;
use futures::StreamExt;
use tokio::sync::{Mutex, RwLock};
use tokio::task;

// ---------------------------------------------------------------------------
// MockProvider: configurable LLM provider for testing
// ---------------------------------------------------------------------------

/// A configurable mock LLM provider that simulates the ChatRepository /
/// ProviderService boundary without any network calls.
#[derive(Clone)]
struct MockProvider {
    responses: Arc<RwLock<Vec<MockResponse>>>,
    call_count: Arc<Mutex<usize>>,
    error_on_call: Arc<Mutex<Option<usize>>>,
    drop_on_call: Arc<Mutex<Option<usize>>>,
    name: String,
}

#[derive(Clone, Debug)]
struct MockResponse {
    content: Option<String>,
    tool_calls: Vec<ToolCallFull>,
    finish_reason: FinishReason,
    latency: Duration,
    usage: Usage,
}

impl MockResponse {
    fn text(text: impl Into<String>) -> Self {
        Self {
            content: Some(text.into()),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            latency: Duration::ZERO,
            usage: Usage::default(),
        }
    }

    fn text_with_latency(text: impl Into<String>, latency: Duration) -> Self {
        Self { latency, ..Self::text(text) }
    }

    fn tool_call(name: &str, args: &str) -> Self {
        Self {
            content: Some(String::new()),
            tool_calls: vec![ToolCallFull {
                name: ToolName::new(name),
                call_id: Some(ToolCallId::generate()),
                arguments: ToolCallArguments::from_json(args),
                thought_signature: None,
            }],
            finish_reason: FinishReason::ToolCalls,
            latency: Duration::ZERO,
            usage: Usage::default(),
        }
    }

    fn text_with_usage(text: impl Into<String>, total_tokens: usize) -> Self {
        Self {
            content: Some(text.into()),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            latency: Duration::ZERO,
            usage: Usage {
                prompt_tokens: TokenCount::Actual(total_tokens / 2),
                completion_tokens: TokenCount::Actual(total_tokens / 2),
                total_tokens: TokenCount::Actual(total_tokens),
                cached_tokens: TokenCount::Actual(0),
                cost: None,
            },
        }
    }
}

impl MockProvider {
    fn new(name: &str) -> Self {
        Self {
            responses: Arc::new(RwLock::new(Vec::new())),
            call_count: Arc::new(Mutex::new(0)),
            error_on_call: Arc::new(Mutex::new(None)),
            drop_on_call: Arc::new(Mutex::new(None)),
            name: name.to_string(),
        }
    }

    async fn push_response(&self, response: MockResponse) {
        self.responses.write().await.push(response);
    }

    async fn fail_on_call(&self, n: usize) {
        *self.error_on_call.lock().await = Some(n);
    }

    async fn drop_on_call(&self, n: usize) {
        *self.drop_on_call.lock().await = Some(n);
    }

    async fn get_call_count(&self) -> usize {
        *self.call_count.lock().await
    }

    /// Simulate the ChatRepository::chat boundary.
    async fn chat(
        &self,
        _context: &Context,
    ) -> anyhow::Result<forge_stream::MpscStream<anyhow::Result<ChatCompletionMessage>>> {
        let mut count = self.call_count.lock().await;
        *count += 1;
        let current_call = *count;
        drop(count);

        if let Some(fail_at) = *self.error_on_call.lock().await
            && current_call == fail_at
        {
            return Err(anyhow::anyhow!(
                "Mock provider '{}' failed on call #{}",
                self.name,
                current_call
            ));
        }

        let should_drop = {
            let drop_at = self.drop_on_call.lock().await;
            drop_at.is_some_and(|n| current_call == n)
        };

        let response = {
            let mut queue = self.responses.write().await;
            if !queue.is_empty() {
                Some(queue.remove(0))
            } else {
                None
            }
        };

        let response =
            response.unwrap_or_else(|| MockResponse::text("No mock response configured"));

        if !response.latency.is_zero() {
            tokio::time::sleep(response.latency).await;
        }

        let msg = ChatCompletionMessage {
            content: response.content.map(Content::full),
            thought_signature: None,
            reasoning: None,
            reasoning_details: None,
            tool_calls: response
                .tool_calls
                .into_iter()
                .map(ToolCall::Full)
                .collect(),
            finish_reason: Some(response.finish_reason),
            usage: Some(response.usage),
            phase: None,
        };

        let stream = forge_stream::MpscStream::spawn(move |tx| async move {
            if should_drop {
                // Simulate mid-stream failure: drop sender without sending anything.
                // The receiver will see Stream ended (None), which call_provider
                // interprets as an empty stream error, triggering fallback.
                drop(tx);
            } else {
                let _ = tx.send(Ok(msg)).await;
            }
        });

        Ok(stream)
    }
}

// ---------------------------------------------------------------------------
// Agent Loop Simulator
// ---------------------------------------------------------------------------

/// Simulates the core agent loop pattern that forge_api orchestrates.
struct AgentLoop {
    providers: Vec<MockProvider>,
    max_iterations: usize,
    tool_executor: Box<dyn Fn(&ToolCallFull) -> String + Send + Sync>,
}

impl AgentLoop {
    fn new(tool_executor: Box<dyn Fn(&ToolCallFull) -> String + Send + Sync>) -> Self {
        Self { providers: vec![], max_iterations: 10, tool_executor }
    }

    fn with_providers(mut self, providers: Vec<MockProvider>) -> Self {
        self.providers = providers;
        self
    }

    fn with_max_iterations(mut self, n: usize) -> Self {
        self.max_iterations = n;
        self
    }

    async fn run(&self, prompt: &str) -> (String, usize, Context) {
        let model = ModelId::new("mock-model");
        let mut context = Context::default()
            .add_message(ContextMessage::system("You are a helpful assistant."))
            .add_message(ContextMessage::user(prompt, Some(model)));

        let mut iterations = 0;

        while iterations < self.max_iterations {
            iterations += 1;
            let response = self.call_provider(&context).await;
            let msg = match response {
                Ok(msg) => msg,
                Err(e) => {
                    panic!("All providers failed: {e}");
                }
            };

            let tool_calls: Vec<ToolCallFull> = msg
                .tool_calls
                .iter()
                .filter_map(|tc| tc.as_full().cloned())
                .collect();
            let content_text = msg
                .content
                .as_ref()
                .map(|c| c.as_str().to_string())
                .unwrap_or_default();
            let usage = msg.usage.unwrap_or_default();

            if tool_calls.is_empty() {
                let return_text = content_text.clone();
                context = context.append_message(
                    content_text,
                    msg.thought_signature,
                    None,
                    None,
                    usage,
                    vec![],
                    msg.phase,
                );
                return (return_text, iterations, context);
            }

            let tool_records: Vec<(ToolCallFull, ToolResult)> = tool_calls
                .iter()
                .map(|tc| {
                    let output = (self.tool_executor)(tc);
                    let result = ToolResult::new(tc.name.clone())
                        .call_id(tc.call_id.clone())
                        .success(output);
                    (tc.clone(), result)
                })
                .collect();

            context = context.append_message(
                content_text,
                msg.thought_signature,
                None,
                None,
                usage,
                tool_records,
                msg.phase,
            );
        }

        let last_content = context
            .messages
            .iter()
            .rev()
            .find_map(|m| m.content().map(|s| s.to_string()))
            .unwrap_or_default();

        (last_content, iterations, context)
    }

    async fn call_provider(&self, context: &Context) -> anyhow::Result<ChatCompletionMessage> {
        let mut last_error: Option<anyhow::Error> = None;

        for provider in &self.providers {
            match provider.chat(context).await {
                Ok(mut stream) => {
                    if let Some(result) = stream.next().await {
                        match result {
                            Ok(msg) => return Ok(msg),
                            Err(e) => {
                                last_error = Some(e);
                                continue;
                            }
                        }
                    } else {
                        last_error = Some(anyhow::anyhow!(
                            "Provider '{}' returned empty stream",
                            provider.name
                        ));
                        continue;
                    }
                }
                Err(e) => {
                    last_error = Some(e);
                    continue;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("No providers configured")))
    }
}

fn mock_tool_executor(call: &ToolCallFull) -> String {
    match call.name.as_str() {
        "shell" => {
            let args = call.arguments.parse().unwrap_or_default();
            let cmd = args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("echo 'no command'");
            format!("Mock shell output for: {cmd}")
        }
        "read" => {
            let args = call.arguments.parse().unwrap_or_default();
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            format!("Mock file content of: {path}")
        }
        "write" => {
            let args = call.arguments.parse().unwrap_or_default();
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            format!("Successfully wrote to: {path}")
        }
        "fs_search" => "Mock search results: found 3 matches".to_string(),
        "patch" => "Mock patch applied successfully".to_string(),
        _ => format!("Mock tool output for: {}", call.name),
    }
}

// ===========================================================================
// Test 1: Full Agent Loop
// ===========================================================================

#[tokio::test]
async fn test_full_agent_loop_with_tool_call() {
    let provider = MockProvider::new("primary");
    provider
        .push_response(MockResponse::tool_call("shell", r#"{"command": "ls -la"}"#))
        .await;
    provider
        .push_response(MockResponse::text(
            "The directory listing shows 5 files and 2 directories.",
        ))
        .await;

    let agent = AgentLoop::new(Box::new(mock_tool_executor)).with_providers(vec![provider.clone()]);

    let (answer, iterations, context) = agent.run("What's in the current directory?").await;

    assert_eq!(
        answer,
        "The directory listing shows 5 files and 2 directories."
    );
    assert_eq!(iterations, 2);
    assert!(context.tool_call_count() >= 1);
    assert!(context.messages.len() >= 4);
    assert_eq!(provider.get_call_count().await, 2);
}

// ===========================================================================
// Test 2: Multi-turn tool calls
// ===========================================================================

#[tokio::test]
async fn test_agent_loop_multi_turn_tool_calls() {
    let provider = MockProvider::new("primary");
    provider
        .push_response(MockResponse::tool_call(
            "read",
            r#"{"path": "src/main.rs"}"#,
        ))
        .await;
    provider
        .push_response(MockResponse::tool_call(
            "fs_search",
            r#"{"pattern": "fn main"}"#,
        ))
        .await;
    provider
        .push_response(MockResponse::tool_call(
            "patch",
            r#"{"path": "src/main.rs", "search": "old", "content": "new"}"#,
        ))
        .await;
    provider
        .push_response(MockResponse::text(
            "I read the file, searched for the pattern, and applied the patch.",
        ))
        .await;

    let agent = AgentLoop::new(Box::new(mock_tool_executor)).with_providers(vec![provider.clone()]);

    let (answer, iterations, _context) = agent.run("Fix the main.rs file").await;
    assert_eq!(iterations, 4);
    assert!(answer.contains("patch"));
    assert_eq!(provider.get_call_count().await, 4);
}

// ===========================================================================
// Test 3: Error Recovery
// ===========================================================================

#[tokio::test]
async fn test_error_recovery_fallback_to_next_provider() {
    let primary = MockProvider::new("primary");
    let fallback = MockProvider::new("fallback");
    primary.fail_on_call(1).await;
    fallback
        .push_response(MockResponse::text("Response from fallback provider"))
        .await;

    let agent = AgentLoop::new(Box::new(mock_tool_executor))
        .with_providers(vec![primary.clone(), fallback.clone()]);

    let (answer, _iterations, _context) = agent.run("Hello from fallback test").await;
    assert_eq!(answer, "Response from fallback provider");
    assert_eq!(primary.get_call_count().await, 1);
    assert_eq!(fallback.get_call_count().await, 1);
}

#[tokio::test]
async fn test_error_recovery_all_providers_fail() {
    let primary = MockProvider::new("primary");
    let fallback = MockProvider::new("fallback");
    primary.fail_on_call(1).await;
    fallback.fail_on_call(1).await;

    let result = std::panic::AssertUnwindSafe(async {
        let agent =
            AgentLoop::new(Box::new(mock_tool_executor)).with_providers(vec![primary, fallback]);
        agent.run("This will fail").await
    });
    let caught = tokio::task::spawn(result).await;
    assert!(
        caught.is_err(),
        "Task should have panicked when all providers failed"
    );
}

// ===========================================================================
// Test 4: Mid-Stream Drop Recovery
// ===========================================================================

#[tokio::test]
async fn test_mid_stream_drop_recovery() {
    let primary = MockProvider::new("primary");
    let fallback = MockProvider::new("fallback");
    primary.drop_on_call(1).await;
    fallback
        .push_response(MockResponse::text("Recovered after stream drop"))
        .await;

    let agent = AgentLoop::new(Box::new(mock_tool_executor))
        .with_providers(vec![primary.clone(), fallback.clone()]);

    let (answer, _iterations, _context) = agent.run("Test stream drop recovery").await;
    assert_eq!(answer, "Recovered after stream drop");
    assert_eq!(primary.get_call_count().await, 1);
    assert_eq!(fallback.get_call_count().await, 1);
}

// ===========================================================================
// Test 5: Context Window Limits
// ===========================================================================

#[tokio::test]
async fn test_context_window_limit_triggers_compaction() {
    const CONTEXT_WINDOW_LIMIT: usize = 50;

    let provider = MockProvider::new("primary");
    for i in 0..5 {
        let text = format!(
            "Response {i}: This is a very long response designed to rapidly fill the context window and trigger the compaction threshold."
        );
        provider
            .push_response(MockResponse::text_with_usage(&text, 50))
            .await;
    }

    let _agent =
        AgentLoop::new(Box::new(mock_tool_executor)).with_providers(vec![provider.clone()]);

    let model = ModelId::new("mock-model");
    let mut context = Context::default()
        .add_message(ContextMessage::system("You are a helpful assistant."))
        .add_message(ContextMessage::user("Tell me something long", Some(model)));

    let mut compaction_triggered = false;

    for _i in 0..5 {
        let mut stream = provider.chat(&context).await.unwrap();
        let msg = stream.next().await.unwrap().unwrap();
        let content_text = msg
            .content
            .as_ref()
            .map(|c| c.as_str().to_string())
            .unwrap_or_default();
        let usage = msg.usage.unwrap_or_default();
        context = context.append_message(content_text, None, None, None, usage, vec![], None);

        let approx_tokens = context.token_count_approx();
        if approx_tokens > CONTEXT_WINDOW_LIMIT && !compaction_triggered {
            let system_messages: Vec<_> = context
                .messages
                .iter()
                .filter(|m| m.has_role(Role::System))
                .cloned()
                .collect();
            let recent_count = 2.min(context.messages.len());
            let recent: Vec<_> = context
                .messages
                .iter()
                .rev()
                .take(recent_count)
                .cloned()
                .collect();
            let mut compacted = Context::default().tools(context.tools.clone());
            compacted.conversation_id = context.conversation_id;
            for msg in system_messages {
                compacted = compacted.add_entry(msg);
            }
            for msg in recent.into_iter().rev() {
                compacted = compacted.add_entry(msg);
            }
            context = compacted;
            compaction_triggered = true;
        }
    }

    assert!(compaction_triggered);
    // After compaction, context should be smaller than peak (pre-compaction was >50 tokens)
    let final_approx = context.token_count_approx();
    assert!(
        final_approx < 200,
        "After compaction, context tokens ({final_approx}) should be reduced from peak"
    );
}

#[tokio::test]
async fn test_context_token_count_tracking() {
    let provider = MockProvider::new("primary");
    provider
        .push_response(MockResponse::text_with_usage("First response", 150))
        .await;
    provider
        .push_response(MockResponse::text_with_usage("Second response", 300))
        .await;

    let agent = AgentLoop::new(Box::new(mock_tool_executor)).with_providers(vec![provider]);

    let (_answer, _iterations, context) = agent.run("Start a conversation").await;
    let accumulated = context.accumulate_usage();
    assert!(accumulated.is_some());
}

// ===========================================================================
// Test 6: Concurrent Request Handling
// ===========================================================================

#[tokio::test]
async fn test_concurrent_requests() {
    let provider = MockProvider::new("concurrent");
    for i in 0..5 {
        provider
            .push_response(MockResponse::text(format!("Response for conversation {i}")))
            .await;
    }

    let provider = Arc::new(provider);
    let mut handles = Vec::new();

    for i in 0..5 {
        let p = provider.clone();
        let prompt = format!("Question {i}");
        handles.push(task::spawn(async move {
            let agent = AgentLoop::new(Box::new(mock_tool_executor))
                .with_providers(vec![p.as_ref().clone()]);
            let (answer, iterations, _context) = agent.run(&prompt).await;
            (i, answer, iterations)
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await.unwrap());
    }
    results.sort_by_key(|r| r.0);

    for (i, answer, iterations) in results {
        assert_eq!(answer, format!("Response for conversation {i}"));
        assert_eq!(iterations, 1);
    }
    assert_eq!(provider.get_call_count().await, 5);
}

#[tokio::test]
async fn test_concurrent_requests_with_mixed_tool_and_text() {
    // Use separate providers for each concurrent task to avoid response
    // queue interleaving.
    let p0 = MockProvider::new("mix-conv0");
    p0.push_response(MockResponse::text("Simple answer")).await;
    let p1 = MockProvider::new("mix-conv1");
    p1.push_response(MockResponse::tool_call("shell", r#"{"command": "pwd"}"#))
        .await;
    p1.push_response(MockResponse::text("Working directory is /home"))
        .await;
    let p2 = MockProvider::new("mix-conv2");
    p2.push_response(MockResponse::text("Another simple answer"))
        .await;

    let mut handles = Vec::new();

    handles.push(task::spawn(async move {
        let agent = AgentLoop::new(Box::new(mock_tool_executor)).with_providers(vec![p0]);
        let (answer, iterations, _) = agent.run("Simple question").await;
        assert_eq!(answer, "Simple answer");
        assert_eq!(iterations, 1);
    }));
    handles.push(task::spawn(async move {
        let agent = AgentLoop::new(Box::new(mock_tool_executor)).with_providers(vec![p1]);
        let (answer, iterations, _) = agent.run("Run a command").await;
        assert_eq!(answer, "Working directory is /home");
        assert_eq!(iterations, 2);
    }));
    handles.push(task::spawn(async move {
        let agent = AgentLoop::new(Box::new(mock_tool_executor)).with_providers(vec![p2]);
        let (answer, iterations, _) = agent.run("Another question").await;
        assert_eq!(answer, "Another simple answer");
        assert_eq!(iterations, 1);
    }));

    for handle in handles {
        handle.await.unwrap();
    }
}

// ===========================================================================
// Test 7: Response Latency
// ===========================================================================

#[tokio::test]
async fn test_mock_provider_response_latency() {
    let provider = MockProvider::new("latency");
    provider
        .push_response(MockResponse::text_with_latency(
            "Delayed response",
            Duration::from_millis(50),
        ))
        .await;

    let start = Instant::now();
    let agent = AgentLoop::new(Box::new(mock_tool_executor)).with_providers(vec![provider]);
    let (answer, _, _) = agent.run("Test latency").await;
    let elapsed = start.elapsed();

    assert_eq!(answer, "Delayed response");
    assert!(
        elapsed >= Duration::from_millis(45),
        "Response should respect latency (elapsed: {elapsed:?})"
    );
}

// ===========================================================================
// Test 8: Serialization Roundtrip
// ===========================================================================

#[tokio::test]
async fn test_chat_request_serialization_roundtrip() {
    let conv_id = ConversationId::generate();
    let event = Event::new(EventValue::text("Hello, world!"));
    let request = ChatRequest::new(event, conv_id);

    let json = serde_json::to_string(&request).unwrap();
    let deserialized: ChatRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(request.conversation_id, deserialized.conversation_id);
    assert_eq!(request.event.id, deserialized.event.id);
    assert_eq!(request.event.value, deserialized.event.value);
}

#[tokio::test]
async fn test_tool_call_full_serialization_roundtrip() {
    let tool_call = ToolCallFull {
        name: ToolName::new("shell"),
        call_id: Some(ToolCallId::new("test-call-id")),
        arguments: ToolCallArguments::from_json(r#"{"command": "ls"}"#),
        thought_signature: None,
    };

    let json = serde_json::to_string(&tool_call).unwrap();
    let deserialized: ToolCallFull = serde_json::from_str(&json).unwrap();

    assert_eq!(tool_call.name, deserialized.name);
    assert_eq!(tool_call.call_id, deserialized.call_id);
    let original_args = tool_call.arguments.parse().unwrap();
    let deserialized_args = deserialized.arguments.parse().unwrap();
    assert_eq!(original_args, deserialized_args);
}

#[tokio::test]
async fn test_conversation_creation_and_context_building() {
    let mut conv = Conversation::generate();
    assert!(conv.is_empty());
    assert_eq!(conv.len(), 0);

    let model = ModelId::new("test-model");
    let context = Context::default()
        .add_message(ContextMessage::system("System prompt"))
        .add_message(ContextMessage::user("Hello", Some(model)))
        .add_message(ContextMessage::assistant("Hi there!", None, None, None));

    conv.context = Some(context);
    assert!(!conv.is_empty());
    assert_eq!(conv.len(), 3);
    assert!(conv.token_count().is_some());
}

// ===========================================================================
// Test 9: Usage Accumulation
// ===========================================================================

#[tokio::test]
async fn test_usage_accumulation_across_turns() {
    let provider = MockProvider::new("usage-accum");
    provider
        .push_response(MockResponse::text_with_usage("Turn 1", 100))
        .await;
    provider
        .push_response(MockResponse::text_with_usage("Turn 2", 200))
        .await;
    provider
        .push_response(MockResponse::text_with_usage("Turn 3", 150))
        .await;

    let agent = AgentLoop::new(Box::new(mock_tool_executor)).with_providers(vec![provider]);
    let (_answer, _iterations, context) = agent.run("Multi-turn usage test").await;

    let accumulated = context.accumulate_usage().unwrap();
    assert!(*accumulated.total_tokens > 0);
}

// ===========================================================================
// Test 10: Context Message Ordering
// ===========================================================================

#[tokio::test]
async fn test_context_message_ordering_preserved() {
    let provider = MockProvider::new("ordering");
    provider
        .push_response(MockResponse::tool_call("shell", r#"{"command": "pwd"}"#))
        .await;
    provider.push_response(MockResponse::text("Done")).await;

    let agent = AgentLoop::new(Box::new(mock_tool_executor)).with_providers(vec![provider]);
    let (_answer, _, context) = agent.run("Run and finish").await;

    let roles: Vec<String> = context
        .messages
        .iter()
        .map(|m| {
            {
                if m.has_role(Role::System) {
                    "system"
                } else if m.has_role(Role::User) {
                    "user"
                } else if m.has_role(Role::Assistant) {
                    "assistant"
                } else if m.has_tool_result() {
                    "tool"
                } else {
                    "other"
                }
            }
            .to_string()
        })
        .collect();

    assert_eq!(roles[0], "system");
    assert_eq!(roles[1], "user");
    assert_eq!(roles[2], "assistant");
    assert_eq!(roles[3], "tool");
    assert_eq!(roles[4], "assistant");
}

// ===========================================================================
// Test 11: Max Iterations Guard
// ===========================================================================

#[tokio::test]
async fn test_max_iterations_prevents_infinite_loop() {
    let provider = MockProvider::new("infinite-loop");
    for _ in 0..20 {
        provider
            .push_response(MockResponse::tool_call(
                "shell",
                r#"{"command": "echo loop"}"#,
            ))
            .await;
    }

    let agent = AgentLoop::new(Box::new(mock_tool_executor))
        .with_providers(vec![provider])
        .with_max_iterations(3);

    let (_answer, iterations, _context) = agent.run("Loop forever").await;
    assert!(
        iterations <= 3,
        "Should be bounded by max_iterations (got {iterations})"
    );
}

// ===========================================================================
// Test 12: Empty Provider Queue
// ===========================================================================

#[tokio::test]
async fn test_empty_provider_queue_returns_fallback() {
    let provider = MockProvider::new("empty");
    let agent = AgentLoop::new(Box::new(mock_tool_executor)).with_providers(vec![provider]);

    let (answer, iterations, _) = agent.run("Empty queue test").await;
    assert_eq!(answer, "No mock response configured");
    assert_eq!(iterations, 1);
}
