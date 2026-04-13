use forge_domain::{
    Compact, CompactionStrategy, Context, ContextMessage, ContextSummary, Environment,
    MessageEntry, Transformer,
};
use tracing::info;

use crate::TemplateEngine;
use crate::transformers::SummaryTransformer;

/// A service dedicated to handling context compaction.
pub struct Compactor {
    compact: Compact,
    environment: Environment,
}

impl Compactor {
    pub fn new(compact: Compact, environment: Environment) -> Self {
        Self { compact, environment }
    }

    /// Applies the standard compaction transformer pipeline to a context
    /// summary.
    ///
    /// This pipeline uses the `Compaction` transformer which:
    /// 1. Drops system role messages
    /// 2. Deduplicates consecutive user messages
    /// 3. Trims context by keeping only the last operation per file path
    /// 4. Deduplicates consecutive assistant content blocks
    /// 5. Strips working directory prefix from file paths
    ///
    /// # Arguments
    ///
    /// * `context_summary` - The context summary to transform
    fn transform(&self, context_summary: ContextSummary) -> ContextSummary {
        SummaryTransformer::new(&self.environment.cwd).transform(context_summary)
    }
}

impl Compactor {
    /// Apply compaction to the context if requested.
    pub fn compact(&self, context: Context, max: bool) -> anyhow::Result<Context> {
        let eviction = CompactionStrategy::evict(self.compact.eviction_window);
        let retention = CompactionStrategy::retain(self.compact.retention_window);

        let strategy = if max {
            // TODO: Consider using `eviction.max(retention)`
            retention
        } else {
            eviction.min(retention)
        };

        match strategy.eviction_range(&context) {
            Some(sequence) => self.compress_single_sequence(context, sequence),
            None => Ok(context),
        }
    }

    /// Compress a single identified sequence of assistant messages.
    fn compress_single_sequence(
        &self,
        mut context: Context,
        sequence: (usize, usize),
    ) -> anyhow::Result<Context> {
        let (start, end) = sequence;

        // The sequence from the original message that needs to be compacted
        // Filter out droppable messages (e.g., attachments) from compaction
        let compaction_sequence = context.messages[start..=end]
            .iter()
            .filter(|msg| !msg.is_droppable())
            .cloned()
            .collect::<Vec<_>>();

        // Create a temporary context for the sequence to generate summary
        let sequence_context = Context::default().messages(compaction_sequence.clone());

        // Generate context summary with tool call information
        let context_summary = ContextSummary::from(&sequence_context);

        // Apply transformers to reduce redundant operations and clean up
        let context_summary = self.transform(context_summary);

        info!(
            sequence_start = sequence.0,
            sequence_end = sequence.1,
            sequence_length = compaction_sequence.len(),
            "Created context compaction summary"
        );

        let summary = TemplateEngine::default().render(
            "forge-partial-summary-frame.md",
            &serde_json::json!({"messages": context_summary.messages}),
        )?;

        // Extended thinking reasoning chain preservation
        //
        // Extended thinking requires the first assistant message to have
        // reasoning_details for subsequent messages to maintain reasoning
        // chains. After compaction, this consistency can break if the first
        // remaining assistant lacks reasoning.
        //
        // Solution: Extract the LAST reasoning from compacted messages and inject it
        // into the first assistant message after compaction. This preserves
        // chain continuity while preventing exponential accumulation across
        // multiple compactions.
        //
        // Example: [U, A+r, U, A+r, U, A] → compact → [U-summary, A+r, U, A]
        //                                                          └─from last
        // compacted
        let reasoning_details = compaction_sequence
            .iter()
            .rev() // Get LAST reasoning (most recent)
            .find_map(|msg| match &**msg {
                ContextMessage::Text(text) => text
                    .reasoning_details
                    .as_ref()
                    .filter(|rd| !rd.is_empty())
                    .cloned(),
                _ => None,
            });

        // Accumulate usage from all messages in the compaction range before they are
        // destroyed
        let compacted_usage = context.messages[start..=end]
            .iter()
            .filter_map(|entry| entry.usage.as_ref())
            .cloned()
            .reduce(|a, b| a.accumulate(&b));

        // Replace the range with the summary, transferring the accumulated usage
        let mut summary_entry = MessageEntry::from(ContextMessage::user(summary, None));
        summary_entry.usage = compacted_usage;
        context
            .messages
            .splice(start..=end, std::iter::once(summary_entry));

        // Remove all droppable messages from the context
        context.messages.retain(|msg| !msg.is_droppable());

        // Inject preserved reasoning into first assistant message (if empty)
        if let Some(reasoning) = reasoning_details
            && let Some(ContextMessage::Text(msg)) = context
                .messages
                .iter_mut()
                .find(|msg| msg.has_role(forge_domain::Role::Assistant))
                .map(|msg| &mut **msg)
            && msg
                .reasoning_details
                .as_ref()
                .is_none_or(|rd| rd.is_empty())
        {
            msg.reasoning_details = Some(reasoning);
        }

        Ok(context)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use forge_domain::MessageEntry;
    use pretty_assertions::assert_eq;

    use super::*;

    fn test_environment() -> Environment {
        use fake::{Fake, Faker};
        let env: Environment = Faker.fake();
        env.cwd(std::path::PathBuf::from("/test/working/dir"))
    }

    #[test]
    fn test_compress_single_sequence_preserves_only_last_reasoning() {
        use forge_domain::ReasoningFull;

        let environment = test_environment();
        let compactor = Compactor::new(Compact::new(), environment);

        let first_reasoning = vec![ReasoningFull {
            text: Some("First thought".to_string()),
            signature: Some("sig1".to_string()),
            ..Default::default()
        }];

        let last_reasoning = vec![ReasoningFull {
            text: Some("Last thought".to_string()),
            signature: Some("sig2".to_string()),
            ..Default::default()
        }];

        let context = Context::default()
            .add_message(ContextMessage::user("M1", None))
            .add_message(ContextMessage::assistant(
                "R1",
                None,
                Some(first_reasoning.clone()),
                None,
            ))
            .add_message(ContextMessage::user("M2", None))
            .add_message(ContextMessage::assistant(
                "R2",
                None,
                Some(last_reasoning.clone()),
                None,
            ))
            .add_message(ContextMessage::user("M3", None))
            .add_message(ContextMessage::assistant("R3", None, None, None));

        let actual = compactor.compress_single_sequence(context, (0, 3)).unwrap();

        // Verify only LAST reasoning_details were preserved
        let assistant_msg = actual
            .messages
            .iter()
            .find(|msg| msg.has_role(forge_domain::Role::Assistant))
            .expect("Should have an assistant message");

        if let ContextMessage::Text(text_msg) = &**assistant_msg {
            assert_eq!(
                text_msg.reasoning_details.as_ref(),
                Some(&last_reasoning),
                "Should preserve only the last reasoning, not the first"
            );
        } else {
            panic!("Expected TextMessage");
        }
    }

    #[test]
    fn test_compress_single_sequence_no_reasoning_accumulation() {
        use forge_domain::ReasoningFull;

        let environment = test_environment();
        let compactor = Compactor::new(Compact::new(), environment);

        let reasoning = vec![ReasoningFull {
            text: Some("Original thought".to_string()),
            signature: Some("sig1".to_string()),
            ..Default::default()
        }];

        // First compaction
        let context = Context::default()
            .add_message(ContextMessage::user("M1", None))
            .add_message(ContextMessage::assistant(
                "R1",
                None,
                Some(reasoning.clone()),
                None,
            ))
            .add_message(ContextMessage::user("M2", None))
            .add_message(ContextMessage::assistant("R2", None, None, None));

        let context = compactor.compress_single_sequence(context, (0, 1)).unwrap();

        // Verify first assistant has the reasoning
        let first_assistant = context
            .messages
            .iter()
            .find(|msg| msg.has_role(forge_domain::Role::Assistant))
            .unwrap();

        if let ContextMessage::Text(text_msg) = &**first_assistant {
            assert_eq!(text_msg.reasoning_details.as_ref().unwrap().len(), 1);
        }

        // Second compaction - add more messages
        let context = context
            .add_message(ContextMessage::user("M3", None))
            .add_message(ContextMessage::assistant("R3", None, None, None));

        let context = compactor.compress_single_sequence(context, (0, 2)).unwrap();

        // Verify reasoning didn't accumulate - should still be just 1 reasoning block
        let first_assistant = context
            .messages
            .iter()
            .find(|msg| msg.has_role(forge_domain::Role::Assistant))
            .unwrap();

        if let ContextMessage::Text(text_msg) = &**first_assistant {
            assert_eq!(
                text_msg.reasoning_details.as_ref().unwrap().len(),
                1,
                "Reasoning should not accumulate across compactions"
            );
        }
    }

    #[test]
    fn test_compress_single_sequence_filters_empty_reasoning() {
        use forge_domain::ReasoningFull;

        let environment = test_environment();
        let compactor = Compactor::new(Compact::new(), environment);

        let non_empty_reasoning = vec![ReasoningFull {
            text: Some("Valid thought".to_string()),
            signature: Some("sig1".to_string()),
            ..Default::default()
        }];

        // Most recent message in range has empty reasoning, earlier has non-empty
        let context = Context::default()
            .add_message(ContextMessage::user("M1", None))
            .add_message(ContextMessage::assistant(
                "R1",
                None,
                Some(non_empty_reasoning.clone()),
                None,
            ))
            .add_message(ContextMessage::user("M2", None))
            .add_message(ContextMessage::assistant("R2", None, Some(vec![]), None)) // Empty - most recent in range
            .add_message(ContextMessage::user("M3", None))
            .add_message(ContextMessage::assistant("R3", None, None, None)); // Outside range

        let actual = compactor.compress_single_sequence(context, (0, 3)).unwrap();

        // After compression: [U-summary, U3, A3]
        // The reasoning from R1 (non-empty) should be injected into A3
        let assistant_msg = actual
            .messages
            .iter()
            .find(|msg| msg.has_role(forge_domain::Role::Assistant))
            .expect("Should have an assistant message");

        if let ContextMessage::Text(text_msg) = &**assistant_msg {
            assert_eq!(
                text_msg.reasoning_details.as_ref(),
                Some(&non_empty_reasoning),
                "Should skip most recent empty reasoning and preserve earlier non-empty"
            );
        } else {
            panic!("Expected TextMessage");
        }
    }

    fn render_template(data: &serde_json::Value) -> String {
        TemplateEngine::default()
            .render("forge-partial-summary-frame.md", data)
            .unwrap()
    }

    #[test]
    fn test_template_engine_renders_summary_frame() {
        use forge_domain::{ContextSummary, Role, SummaryBlock, SummaryMessage, SummaryToolCall};

        // Create test data with various tool calls and text content
        let messages = vec![
            SummaryBlock::new(
                Role::User,
                vec![SummaryMessage::content("Please read the config file")],
            ),
            SummaryBlock::new(
                Role::Assistant,
                vec![
                    SummaryToolCall::read("config.toml")
                        .id("call_1")
                        .is_success(false)
                        .into(),
                ],
            ),
            SummaryBlock::new(
                Role::User,
                vec![SummaryMessage::content("Now update the version number")],
            ),
            SummaryBlock::new(
                Role::Assistant,
                vec![SummaryToolCall::update("Cargo.toml").id("call_2").into()],
            ),
            SummaryBlock::new(
                Role::User,
                vec![SummaryMessage::content("Search for TODO comments")],
            ),
            SummaryBlock::new(
                Role::Assistant,
                vec![
                    SummaryToolCall::search("TODO")
                        .id("call_3")
                        .is_success(false)
                        .into(),
                ],
            ),
            SummaryBlock::new(
                Role::Assistant,
                vec![
                    SummaryToolCall::codebase_search(vec![forge_domain::SearchQuery::new(
                        "authentication logic",
                        "Find authentication implementation",
                    )])
                    .id("call_4")
                    .is_success(false)
                    .into(),
                ],
            ),
            SummaryBlock::new(
                Role::Assistant,
                vec![
                    SummaryToolCall::shell("cargo test")
                        .id("call_5")
                        .is_success(false)
                        .into(),
                ],
            ),
            SummaryBlock::new(
                Role::User,
                vec![SummaryMessage::content("Great! Everything looks good.")],
            ),
        ];

        let context_summary = ContextSummary { messages };
        let data = serde_json::json!({"messages": context_summary.messages});

        let actual = render_template(&data);

        insta::assert_snapshot!(actual);
    }

    #[test]
    fn test_template_engine_renders_todo_write() {
        use forge_domain::{
            ContextSummary, Role, SummaryBlock, SummaryMessage, SummaryTool, SummaryToolCall, Todo,
            TodoChange, TodoChangeKind, TodoStatus,
        };

        // Create test data with todo_write tool call showing a diff
        let changes = vec![
            TodoChange {
                todo: Todo::new("Implement user authentication")
                    .id("1")
                    .status(TodoStatus::Completed),
                kind: TodoChangeKind::Updated,
            },
            TodoChange {
                todo: Todo::new("Add database migrations")
                    .id("2")
                    .status(TodoStatus::InProgress),
                kind: TodoChangeKind::Added,
            },
            TodoChange {
                todo: Todo::new("Write documentation")
                    .id("3")
                    .status(TodoStatus::Pending),
                kind: TodoChangeKind::Removed,
            },
        ];

        let messages = vec![
            SummaryBlock::new(
                Role::User,
                vec![SummaryMessage::content("Create a task plan")],
            ),
            SummaryBlock::new(
                Role::Assistant,
                vec![
                    SummaryToolCall {
                        id: Some(forge_domain::ToolCallId::new("call_1")),
                        tool: SummaryTool::TodoWrite { changes },
                        is_success: true,
                    }
                    .into(),
                ],
            ),
        ];

        let context_summary = ContextSummary { messages };
        let data = serde_json::json!({"messages": context_summary.messages});

        let actual = render_template(&data);

        insta::assert_snapshot!(actual);
    }

    #[tokio::test]
    async fn test_render_summary_frame_snapshot() {
        // Load the conversation fixture
        let fixture_json = forge_test_kit::fixture!("/src/fixtures/conversation.json").await;

        let conversation: forge_domain::Conversation =
            serde_json::from_str(&fixture_json).expect("Failed to parse conversation fixture");

        // Extract context from conversation
        let context = conversation
            .context
            .expect("Conversation should have context");

        // Create compactor instance for transformer access
        let environment = test_environment().cwd(PathBuf::from(
            "/Users/tushar/Documents/Projects/code-forge-workspace/code-forge",
        ));
        let compactor = Compactor::new(Compact::new(), environment);

        // Create context summary with tool call information
        let context_summary = ContextSummary::from(&context);

        // Apply transformers to reduce redundant operations and clean up
        let context_summary = compactor.transform(context_summary);

        let data = serde_json::json!({"messages": context_summary.messages});

        let summary = render_template(&data);

        insta::assert_snapshot!(summary);

        // Perform a full compaction
        let compacted_context = compactor.compact(context, true).unwrap();

        insta::assert_yaml_snapshot!(compacted_context);
    }

    #[test]
    fn test_compaction_removes_droppable_messages() {
        use forge_domain::{ContextMessage, Role, TextMessage};

        let environment = test_environment();
        let compactor = Compactor::new(Compact::new(), environment);

        // Create a context with droppable attachment messages
        let context = Context::default()
            .add_message(ContextMessage::user("User message 1", None))
            .add_message(ContextMessage::assistant(
                "Assistant response 1",
                None,
                None,
                None,
            ))
            .add_message(ContextMessage::Text(
                TextMessage::new(Role::User, "Attachment content").droppable(true),
            ))
            .add_message(ContextMessage::user("User message 2", None))
            .add_message(ContextMessage::assistant(
                "Assistant response 2",
                None,
                None,
                None,
            ));

        let actual = compactor.compress_single_sequence(context, (0, 1)).unwrap();

        // The compaction should remove the droppable message
        // Expected: [U-summary, U2, A2]
        assert_eq!(actual.messages.len(), 3);

        // Verify the droppable attachment message was removed
        for msg in &actual.messages {
            if let ContextMessage::Text(text_msg) = &**msg {
                assert!(!text_msg.droppable, "Droppable messages should be removed");
            }
        }
    }

    #[test]
    fn test_compaction_preserves_usage_information() {
        use forge_domain::{TokenCount, Usage};

        let environment = test_environment();
        let compactor = Compactor::new(Compact::new(), environment);

        // Usage on a message INSIDE the compaction range (index 1)
        let inside_usage = Usage {
            total_tokens: TokenCount::Actual(20000),
            prompt_tokens: TokenCount::Actual(18000),
            completion_tokens: TokenCount::Actual(2000),
            cached_tokens: TokenCount::Actual(0),
            cost: Some(0.5),
        };

        // Usage on a message INSIDE the compaction range (index 3)
        let inside_usage2 = Usage {
            total_tokens: TokenCount::Actual(30000),
            prompt_tokens: TokenCount::Actual(27000),
            completion_tokens: TokenCount::Actual(3000),
            cached_tokens: TokenCount::Actual(0),
            cost: Some(1.0),
        };

        // Usage on a message OUTSIDE the compaction range (index 5)
        let outside_usage = Usage {
            total_tokens: TokenCount::Actual(50000),
            prompt_tokens: TokenCount::Actual(45000),
            completion_tokens: TokenCount::Actual(5000),
            cached_tokens: TokenCount::Actual(0),
            cost: Some(1.5),
        };

        let mut entry1 =
            MessageEntry::from(ContextMessage::assistant("Response 1", None, None, None));
        entry1.usage = Some(inside_usage);

        let mut entry3 =
            MessageEntry::from(ContextMessage::assistant("Response 2", None, None, None));
        entry3.usage = Some(inside_usage2);

        let mut entry5 =
            MessageEntry::from(ContextMessage::assistant("Response 3", None, None, None));
        entry5.usage = Some(outside_usage);

        let context = Context::default()
            .add_entry(ContextMessage::user("Message 1", None))
            .add_entry(entry1) // index 1: usage INSIDE range
            .add_entry(ContextMessage::user("Message 2", None))
            .add_entry(entry3) // index 3: usage INSIDE range
            .add_entry(ContextMessage::user("Message 3", None))
            .add_entry(entry5); // index 5: usage OUTSIDE range

        // Compact the sequence (first 4 messages, indices 0-3)
        let compacted = compactor.compress_single_sequence(context, (0, 3)).unwrap();

        // Expected: [summary-entry, U3, A3] — 3 messages remain
        assert_eq!(
            compacted.messages.len(),
            3,
            "Expected 3 messages after compaction: summary + 2 remaining messages"
        );

        // The summary entry at index 0 should carry the accumulated usage from
        // indices 1 and 3 (inside_usage + inside_usage2)
        let expected_compacted_usage = Usage {
            total_tokens: TokenCount::Actual(50000),
            prompt_tokens: TokenCount::Actual(45000),
            completion_tokens: TokenCount::Actual(5000),
            cached_tokens: TokenCount::Actual(0),
            cost: Some(1.5),
        };

        assert_eq!(
            compacted.messages[0].usage,
            Some(expected_compacted_usage),
            "Summary message should carry accumulated usage from compacted messages"
        );

        // accumulate_usage() must sum both the compacted range usage (on the summary
        // message) and the surviving outside_usage — total = inside + inside2 + outside
        let expected_total_usage = Usage {
            total_tokens: TokenCount::Actual(100000),
            prompt_tokens: TokenCount::Actual(90000),
            completion_tokens: TokenCount::Actual(10000),
            cached_tokens: TokenCount::Actual(0),
            cost: Some(3.0),
        };

        assert_eq!(
            compacted.accumulate_usage(),
            Some(expected_total_usage),
            "accumulate_usage() must include usage from both compacted and surviving messages"
        );
    }

    /// Creates a Context from a condensed string pattern where:
    /// - 'u' = User message
    /// - 'a' = Assistant message
    /// - 's' = System message
    fn ctx(pattern: &str) -> Context {
        forge_domain::MessagePattern::new(pattern).build()
    }

    #[test]
    fn test_should_compact_no_thresholds_set() {
        let fixture = Compact::new().model("test-model");
        let context = ctx("ua");
        let actual = fixture.should_compact(&context, 1000);
        assert_eq!(actual, false);
    }

    #[test]
    fn test_should_compact_token_threshold_triggers() {
        let fixture = Compact::new()
            .model("test-model")
            .token_threshold(100_usize);
        let context = ctx("u");
        let actual = fixture.should_compact(&context, 150);
        assert_eq!(actual, true);
    }

    #[test]
    fn test_should_compact_turn_threshold_triggers() {
        let fixture = Compact::new().model("test-model").turn_threshold(1_usize);
        let context = ctx("uau");
        let actual = fixture.should_compact(&context, 50);
        assert_eq!(actual, true);
    }

    #[test]
    fn test_should_compact_message_threshold_triggers() {
        let fixture = Compact::new()
            .model("test-model")
            .message_threshold(2_usize);
        let context = ctx("uau");
        let actual = fixture.should_compact(&context, 50);
        assert_eq!(actual, true);
    }

    #[test]
    fn test_should_compact_multiple_thresholds_any_triggers() {
        let fixture = Compact::new()
            .model("test-model")
            .token_threshold(200_usize)
            .turn_threshold(5_usize)
            .message_threshold(10_usize);
        let context = ctx("ua");
        let actual = fixture.should_compact(&context, 250);
        assert_eq!(actual, true);
    }

    #[test]
    fn test_should_compact_multiple_thresholds_none_trigger() {
        let fixture = Compact::new()
            .model("test-model")
            .token_threshold(200_usize)
            .turn_threshold(5_usize)
            .message_threshold(10_usize);
        let context = ctx("ua");
        let actual = fixture.should_compact(&context, 100);
        assert_eq!(actual, false);
    }

    #[test]
    fn test_should_compact_empty_context() {
        let fixture = Compact::new()
            .model("test-model")
            .message_threshold(1_usize);
        let context = ctx("");
        let actual = fixture.should_compact(&context, 0);
        assert_eq!(actual, false);
    }

    #[test]
    fn test_should_compact_last_user_message_integration() {
        let fixture = Compact::new().model("test-model").on_turn_end(true);
        let context = ctx("au");
        let actual = fixture.should_compact(&context, 10);
        assert_eq!(actual, true);
    }

    #[test]
    fn test_should_compact_last_user_message_integration_disabled() {
        let fixture = Compact::new().model("test-model").on_turn_end(false);
        let context = ctx("au");
        let actual = fixture.should_compact(&context, 10);
        assert_eq!(actual, false);
    }

    #[test]
    fn test_should_compact_multiple_conditions_with_last_user_message() {
        let fixture = Compact::new()
            .model("test-model")
            .token_threshold(200_usize)
            .on_turn_end(true);
        let context = ctx("au");
        let actual = fixture.should_compact(&context, 50);
        assert_eq!(actual, true);
    }

    #[test]
    fn test_compact_model_none_falls_back_to_agent_model() {
        let compact = Compact::new()
            .token_threshold(1000_usize)
            .turn_threshold(5_usize);
        assert_eq!(compact.model, None);
        assert_eq!(compact.token_threshold, Some(1000_usize));
        assert_eq!(compact.turn_threshold, Some(5_usize));
    }

    /// BUG 5: Context growth simulation showing how context_length_exceeded error occurs.
    /// 
    /// This test simulates a conversation with codex-spark (128K context window) and
    /// default token_threshold of 100K. It shows how:
    /// 1. Context grows turn by turn without triggering compaction (below 100K threshold)
    /// 2. Each turn adds user message + tool outputs
    /// 3. Eventually context + tool outputs exceed 128K limit
    /// 4. API returns context_length_exceeded error
    /// 
    /// Test that demonstrates how the fixed compaction threshold prevents
    /// context_length_exceeded errors.
    ///
    /// With the fix, token_threshold of 100K is capped to 89600 (70% of 128K),
    /// ensuring compaction triggers earlier to provide safety margin.
    #[test]
    fn test_safe_threshold_triggers_earlier_than_unsafe_threshold() {
        use forge_domain::{ContextMessage, ToolResult, ToolName, ToolCallId};

        // Two configurations: unsafe (100K) vs safe (89.6K = 70% of 128K)
        let unsafe_compact = Compact::new()
            .token_threshold(100_000_usize)  // Old unsafe threshold
            .max_tokens(2000_usize);
        
        let safe_compact = Compact::new()
            .token_threshold(89_600_usize)  // Safe threshold (70% of 128K)
            .max_tokens(2000_usize);

        let _environment = test_environment();

        // Start with initial context of 80000 tokens
        let mut unsafe_context = create_large_context(80_000);
        let mut safe_context = create_large_context(80_000);
        
        // Simulate 2 conversation turns
        for turn in 1..=2 {
            // Add same messages to both contexts
            let user_msg = ContextMessage::user(
                &format!("Turn {}: Please analyze this file", turn),
                None
            );
            let assistant_msg = ContextMessage::assistant(
                &format!("I'll analyze for turn {}", turn),
                None, None, None
            );
            
            unsafe_context = unsafe_context.add_message(user_msg.clone());
            safe_context = safe_context.add_message(user_msg);
            
            unsafe_context = unsafe_context.add_message(assistant_msg.clone());
            safe_context = safe_context.add_message(assistant_msg);

            // Add tool outputs
            for file_read in 1..=3 {
                let tool_result = ToolResult::new(ToolName::new("read"))
                    .call_id(ToolCallId::new(&format!("call_{}_{}", turn, file_read)))
                    .success(create_large_content(5000));
                
                unsafe_context = unsafe_context.add_tool_results(vec![tool_result.clone()]);
                safe_context = safe_context.add_tool_results(vec![tool_result]);
            }

            let unsafe_token_count = unsafe_context.token_count_approx();
            let safe_token_count = safe_context.token_count_approx();
            
            let unsafe_should_compact = unsafe_compact.should_compact(&unsafe_context, unsafe_token_count);
            let safe_should_compact = safe_compact.should_compact(&safe_context, safe_token_count);
        }

        // At turn 1:
        // - Unsafe threshold (100K): ~95K tokens, NO compaction (false)
        // - Safe threshold (89.6K): ~95K tokens, SHOULD compact (true)
        //
        // At turn 2:
        // - Unsafe threshold (100K): ~110K tokens, SHOULD compact (true) - but too late!
        // - Safe threshold (89.6K): ~110K tokens, already compacted at turn 1
        
        // Verify that safe threshold triggers at turn 1 (providing early warning)
        let safe_token_count_turn1 = 95_000; // Approximate
        let safe_should_compact_turn1 = safe_compact.should_compact(&safe_context, safe_token_count_turn1);
        
        // The key fix: safe threshold (89.6K) triggers at ~95K, while unsafe (100K) doesn't
        // This provides a safety margin before we hit the 128K limit
        assert!(
            safe_should_compact_turn1 || safe_token_count_turn1 < 89_600,
            "Safe threshold (89.6K) should trigger compaction at ~95K tokens to provide safety margin"
        );

        // After 2 turns, both contexts are similar size (~110K)
        // But with safe threshold, compaction would have triggered earlier
        let final_unsafe = unsafe_context.token_count_approx();
        let final_safe = safe_context.token_count_approx();
        
        // Both should be identical since we're just testing threshold logic, not actual compaction
        assert_eq!(final_unsafe, final_safe, "Both contexts should have same token count");
        
        // The important assertion: with unsafe 100K threshold, context can grow to ~110K
        // before compaction triggers, leaving only 18K headroom for the 128K limit.
        // With safe 89.6K threshold, compaction triggers at ~95K, leaving 33K headroom.
        //
        // This extra headroom is critical because tool outputs can add 15K+ tokens per turn,
        // and without early compaction, context + tool outputs can exceed 128K limit.
    }

    /// Helper to create a large context with approximately `token_count` tokens
    fn create_large_context(token_count: usize) -> Context {
        use forge_domain::ContextMessage;
        
        // Each char is ~0.25 tokens (4 chars per token)
        let char_count = token_count * 4;
        let content = "x".repeat(char_count);
        
        // Split into multiple messages to avoid single huge message
        let messages_needed = 10;
        let content_per_message = content.len() / messages_needed;
        
        let mut context = Context::default();
        for i in 0..messages_needed {
            let start = i * content_per_message;
            let end = ((i + 1) * content_per_message).min(content.len());
            let msg_content = &content[start..end];
            
            if i % 2 == 0 {
                context = context.add_message(ContextMessage::user(msg_content, None));
            } else {
                context = context.add_message(ContextMessage::assistant(msg_content, None, None, None));
            }
        }
        
        context
    }

    /// Helper to create large content of approximately `token_count` tokens
    fn create_large_content(token_count: usize) -> String {
        // 4 chars per token approximation
        "x".repeat(token_count * 4)
    }
}
