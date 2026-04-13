use async_trait::async_trait;
use derive_setters::Setters;
use forge_domain::{
    ContextMessage, Conversation, EventData, EventHandle, RequestPayload, Role, TextMessage,
    ToolCallArguments, ToolName,
};
use forge_template::Element;
use tracing::warn;

use crate::TemplateEngine;

/// Detector for identifying doom loops - when tool calls form repetitive
/// patterns
///
/// This detector analyzes conversation history to identify two types of loops:
/// 1. Consecutive identical calls: [A,A,A,A] - same tool with same arguments
/// 2. Repeating patterns: [A,B,C][A,B,C][A,B,C] - sequence of calls repeating
///
/// Both patterns indicate the agent is stuck in a loop, wasting tokens without
/// making progress.
///
/// Can be used as a hook on `on_request` events to detect doom loops after
/// tool call records from prior turns are already persisted in context.
#[derive(Debug, Clone, Setters)]
pub struct DoomLoopDetector {
    /// Threshold for consecutive identical tool calls before triggering
    /// detection
    threshold: usize,
}

impl Default for DoomLoopDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl DoomLoopDetector {
    const DEFAULT_THRESHOLD: usize = 3;

    /// Creates a new doom loop detector with the default threshold
    pub fn new() -> Self {
        Self { threshold: Self::DEFAULT_THRESHOLD }
    }

    /// Checks conversation history for doom loops using already-recorded tool
    /// call history in context.
    ///
    /// This variant is intended for request-phase hooks, where tool call
    /// results from the previous turn have already been appended to context.
    ///
    /// Returns Some(count) if a doom loop is detected
    pub fn detect_from_conversation(&self, conversation: &Conversation) -> Option<usize> {
        let all_signatures = self.extract_tool_signatures(conversation);

        let (_, count) = self.check_repeating_pattern(&all_signatures)?;

        Some(count)
    }

    fn extract_tool_signatures(
        &self,
        conversation: &Conversation,
    ) -> Vec<(ToolName, ToolCallArguments)> {
        let assistant_messages = conversation
            .context
            .as_ref()
            .map(|ctx| {
                Self::extract_assistant_messages(ctx.messages.iter().map(|entry| &entry.message))
            })
            .unwrap_or_default();

        assistant_messages
            .iter()
            .filter_map(|msg| msg.tool_calls.as_ref())
            .flat_map(|calls| calls.iter())
            .map(|call| (call.name.clone(), call.arguments.clone()))
            .collect()
    }

    /// Checks for repeating patterns at the end of the sequence.
    fn check_repeating_pattern<T>(&self, sequence: &[T]) -> Option<(usize, usize)>
    where
        T: Eq,
    {
        if sequence.is_empty() {
            return None;
        }

        if sequence.len() < self.threshold {
            return None;
        }

        for pattern_length in 1..sequence.len() {
            let complete_repetitions =
                self.count_recent_pattern_repetitions(sequence, pattern_length);

            if complete_repetitions >= self.threshold {
                let pattern_offset = complete_repetitions.checked_mul(pattern_length)?;
                let pattern_start_idx = sequence.len().checked_sub(pattern_offset)?;

                if sequence.get(pattern_start_idx).is_some() {
                    return Some((pattern_start_idx, complete_repetitions));
                }
            }
        }

        None
    }

    /// Counts how many times a pattern of given length repeats at the END of
    /// the sequence
    ///
    /// This works backwards from the most recent calls to find repeating
    /// patterns, which allows detecting new patterns even if earlier
    /// patterns existed. For example, in [1,2,3,1,2,3,4,5,4,5,4,5], this
    /// will detect [4,5] repeating 3 times.
    fn count_recent_pattern_repetitions<T>(&self, sequence: &[T], pattern_length: usize) -> usize
    where
        T: Eq,
    {
        if pattern_length == 0 || sequence.len() < pattern_length {
            return 0;
        }

        // Start from the end and work backwards
        let total_len = sequence.len();
        let mut repetitions = 0;

        // The pattern is defined by the last pattern_length elements
        // For a partial match, we consider it as the start of a new repetition
        let mut check_len = total_len;

        // Special case: if total length is not evenly divisible by pattern_length,
        // we have a partial match at the end
        if !total_len.is_multiple_of(pattern_length) {
            let partial_len = total_len % pattern_length;
            // Check if the partial segment matches the start of what would be the pattern
            // We need to look back to find what the pattern would be
            if total_len < pattern_length + partial_len {
                return 0;
            }

            let pattern_start = total_len - partial_len - pattern_length;
            let pattern_end = pattern_start + pattern_length;
            let partial_start = total_len - partial_len;

            let Some(pattern) = sequence.get(pattern_start..pattern_end) else {
                return 0;
            };
            let Some(partial) = sequence.get(partial_start..total_len) else {
                return 0;
            };
            let Some(pattern_prefix) = pattern.get(..partial_len) else {
                return 0;
            };

            if partial == pattern_prefix {
                repetitions += 1;
                check_len = total_len - partial_len;
            } else {
                // Partial doesn't match, no pattern
                return 0;
            }
        }

        // Now check complete repetitions working backwards
        if check_len < pattern_length {
            return repetitions;
        }

        // The pattern is the last complete chunk
        let pattern_start = check_len - pattern_length;
        let Some(pattern) = sequence.get(pattern_start..check_len) else {
            return repetitions;
        };
        repetitions += 1; // Count the pattern itself

        // Check backwards for more repetitions
        let mut pos = pattern_start;
        while pos >= pattern_length {
            pos -= pattern_length;
            let Some(chunk) = sequence.get(pos..pos + pattern_length) else {
                break;
            };

            if chunk == pattern {
                repetitions += 1;
            } else {
                // Pattern broken, stop counting
                break;
            }
        }

        repetitions
    }

    /// Extracts assistant messages from context messages
    ///
    /// Helper method to filter assistant messages from a conversation context
    pub fn extract_assistant_messages<'a>(
        messages: impl Iterator<Item = &'a ContextMessage> + 'a,
    ) -> Vec<&'a TextMessage> {
        messages
            .filter_map(|msg| {
                if let ContextMessage::Text(text_msg) = msg
                    && text_msg.role == Role::Assistant
                {
                    return Some(text_msg);
                }
                None
            })
            .collect()
    }
}

/// Implementation of EventHandle for DoomLoopDetector
///
/// This allows the detector to run on request events so the previous turn's
/// tool calls and results are already appended in context before reminders are
/// inserted.
#[async_trait]
impl EventHandle<EventData<RequestPayload>> for DoomLoopDetector {
    async fn handle(
        &self,
        event: &EventData<RequestPayload>,
        conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        if let Some(consecutive_calls) = self.detect_from_conversation(conversation) {
            warn!(
                agent_id = %event.agent.id,
                request_count = event.payload.request_count,
                consecutive_calls,
                "Doom loop detected from conversation context before next request"
            );

            if let Some(context) = conversation.context.as_mut() {
                let reminder = TemplateEngine::default().render(
                    "forge-doom-loop-reminder.md",
                    &serde_json::json!({"consecutive_calls": consecutive_calls}),
                )?;
                let content = Element::new("system_reminder").cdata(reminder);
                context
                    .messages
                    .push(ContextMessage::user(content, None).into());
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{
        Context, ContextMessage, ConversationId, MessageEntry, ToolCallArguments, ToolCallFull,
    };
    use pretty_assertions::assert_eq;

    use super::*;

    fn create_assistant_message(tool_call: &ToolCallFull) -> TextMessage {
        TextMessage {
            role: Role::Assistant,
            content: String::new(),
            raw_content: None,
            tool_calls: Some(vec![tool_call.clone()]),
            thought_signature: None,
            model: None,
            reasoning_details: None,
            droppable: false,
            phase: None,
        }
    }

    fn create_conversation_with_messages(messages: Vec<TextMessage>) -> Conversation {
        let context_messages: Vec<MessageEntry> = messages
            .into_iter()
            .map(|msg| MessageEntry::from(ContextMessage::Text(msg)))
            .collect();

        let context = Context::default().messages(context_messages);

        Conversation {
            id: ConversationId::generate(),
            title: None,
            context: Some(context),
            metrics: Default::default(),
            metadata: forge_domain::MetaData::new(chrono::Utc::now()),
        }
    }

    #[test]
    fn test_doom_loop_detector_detects_identical_calls() {
        let detector = DoomLoopDetector::new();

        let tool_call = ToolCallFull::new("read")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file.txt"}"#));

        // Build history with repeated calls
        let msg1 = create_assistant_message(&tool_call);
        let msg2 = create_assistant_message(&tool_call);
        let msg3 = create_assistant_message(&tool_call);
        let conversation = create_conversation_with_messages(vec![msg1, msg2, msg3]);

        // Third call - doom loop detected!
        let actual = detector.detect_from_conversation(&conversation);
        let expected = Some(3);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_doom_loop_detector_no_loop_with_two_calls() {
        let detector = DoomLoopDetector::new();

        let tool_call = ToolCallFull::new("read")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file.txt"}"#));

        // Build history with one call
        let msg1 = create_assistant_message(&tool_call);
        let conversation = create_conversation_with_messages(vec![msg1]);

        // Second call - no loop yet (need 3 for default threshold)
        let actual = detector.detect_from_conversation(&conversation);
        assert_eq!(actual, None);
    }

    #[test]
    fn test_doom_loop_detector_resets_on_different_arguments() {
        let detector = DoomLoopDetector::new();

        let tool_call_1 = ToolCallFull::new("read")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file1.txt"}"#));
        let tool_call_2 = ToolCallFull::new("read")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file2.txt"}"#));

        // Build history with two calls of first arguments, then different
        let msg1 = create_assistant_message(&tool_call_1);
        let msg2 = create_assistant_message(&tool_call_1);
        let msg3 = create_assistant_message(&tool_call_2);
        let conversation = create_conversation_with_messages(vec![msg1, msg2, msg3]);

        // Call with first arguments again - should not detect loop
        let actual = detector.detect_from_conversation(&conversation);
        assert_eq!(actual, None);
    }

    #[test]
    fn test_doom_loop_detector_resets_on_different_tool() {
        let detector = DoomLoopDetector::new();

        let tool_call_1 = ToolCallFull::new("read")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file.txt"}"#));
        let tool_call_2 = ToolCallFull::new("write")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file.txt"}"#));

        // Build history with two same tool calls, then different tool
        let msg1 = create_assistant_message(&tool_call_1);
        let msg2 = create_assistant_message(&tool_call_1);
        let msg3 = create_assistant_message(&tool_call_2);
        let conversation = create_conversation_with_messages(vec![msg1, msg2, msg3]);

        // Call different tool - should not detect loop
        let actual = detector.detect_from_conversation(&conversation);
        assert_eq!(actual, None);
    }

    #[test]
    fn test_doom_loop_detector_custom_threshold() {
        let detector = DoomLoopDetector::new().threshold(2);

        let tool_call = ToolCallFull::new("read")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file.txt"}"#));

        // Build history with one call
        let msg1 = create_assistant_message(&tool_call);
        let msg2 = create_assistant_message(&tool_call);
        let conversation = create_conversation_with_messages(vec![msg1, msg2]);

        // Second call - doom loop detected with threshold of 2!
        let actual = detector.detect_from_conversation(&conversation);
        let expected = Some(2);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_doom_loop_detector_empty_history() {
        let detector = DoomLoopDetector::new();

        // Empty history - first call, no loop
        let conversation = create_conversation_with_messages(vec![]);

        let actual = detector.detect_from_conversation(&conversation);
        assert_eq!(actual, None);
    }

    #[test]
    fn test_extract_assistant_messages() {
        let assistant_msg_1 = TextMessage {
            role: Role::Assistant,
            content: "Response 1".to_string(),
            raw_content: None,
            tool_calls: None,
            thought_signature: None,
            model: None,
            reasoning_details: None,
            droppable: false,
            phase: None,
        };

        let user_msg = TextMessage {
            role: Role::User,
            content: "Question".to_string(),
            raw_content: None,
            tool_calls: None,
            thought_signature: None,
            model: None,
            reasoning_details: None,
            droppable: false,
            phase: None,
        };

        let assistant_msg_2 = TextMessage {
            role: Role::Assistant,
            content: "Response 2".to_string(),
            raw_content: None,
            tool_calls: None,
            thought_signature: None,
            model: None,
            reasoning_details: None,
            droppable: false,
            phase: None,
        };

        let messages = [
            ContextMessage::Text(assistant_msg_1.clone()),
            ContextMessage::Text(user_msg),
            ContextMessage::Text(assistant_msg_2.clone()),
        ];

        let result = DoomLoopDetector::extract_assistant_messages(messages.iter());

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].content, "Response 1");
        assert_eq!(result[1].content, "Response 2");
    }

    #[test]
    fn test_detect_pattern_start_with_integers_for_123_123_123() {
        let detector = DoomLoopDetector::new();
        let fixture = vec![1, 2, 3, 1, 2, 3, 1, 2, 3];

        let actual = detector.check_repeating_pattern(&fixture);
        let expected = Some((0, 3));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_detect_pattern_start_with_integers_detects_recent_suffix_pattern() {
        let detector = DoomLoopDetector::new();
        let fixture = vec![1, 2, 3, 1, 2, 3, 4, 5, 4, 5, 4, 5];

        let actual = detector.check_repeating_pattern(&fixture);
        let expected = Some((6, 3));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_detect_pattern_start_with_integers_detects_consecutive_identical() {
        let detector = DoomLoopDetector::new();
        let fixture = vec![1, 2, 3, 3, 3];

        let actual = detector.check_repeating_pattern(&fixture);
        let expected = Some((2, 3));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_doom_loop_detector_detects_repeating_pattern_123_123_123() {
        let detector = DoomLoopDetector::new();

        let tool_call_1 = ToolCallFull::new("read")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file1.txt"}"#));
        let tool_call_2 = ToolCallFull::new("write")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file2.txt"}"#));
        let tool_call_3 = ToolCallFull::new("patch")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file3.txt"}"#));

        // Build history with pattern [1,2,3][1,2,3][1,2,3]
        let msg1 = create_assistant_message(&tool_call_1);
        let msg2 = create_assistant_message(&tool_call_2);
        let msg3 = create_assistant_message(&tool_call_3);
        let msg4 = create_assistant_message(&tool_call_1);
        let msg5 = create_assistant_message(&tool_call_2);
        let msg6 = create_assistant_message(&tool_call_3);
        let msg7 = create_assistant_message(&tool_call_1);
        let msg8 = create_assistant_message(&tool_call_2);
        let msg9 = create_assistant_message(&tool_call_3);

        let conversation = create_conversation_with_messages(vec![
            msg1, msg2, msg3, msg4, msg5, msg6, msg7, msg8, msg9,
        ]);

        let actual = detector.detect_from_conversation(&conversation);

        // Should detect pattern repetition (3 times)
        assert_eq!(actual, Some(3));
    }

    #[test]
    fn test_doom_loop_detector_detects_repeating_pattern_12_12_12() {
        let detector = DoomLoopDetector::new();

        let tool_call_1 = ToolCallFull::new("read")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file1.txt"}"#));
        let tool_call_2 = ToolCallFull::new("write")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file2.txt"}"#));

        // Build history with pattern [1,2][1,2][1,2]
        let msg1 = create_assistant_message(&tool_call_1);
        let msg2 = create_assistant_message(&tool_call_2);
        let msg3 = create_assistant_message(&tool_call_1);
        let msg4 = create_assistant_message(&tool_call_2);
        let msg5 = create_assistant_message(&tool_call_1);
        let msg6 = create_assistant_message(&tool_call_2);

        let conversation =
            create_conversation_with_messages(vec![msg1, msg2, msg3, msg4, msg5, msg6]);

        let actual = detector.detect_from_conversation(&conversation);

        // Should detect pattern repetition (3 times)
        assert_eq!(actual, Some(3));
    }

    #[test]
    fn test_doom_loop_detector_no_pattern_with_partial_repetition() {
        let detector = DoomLoopDetector::new();

        let tool_call_1 = ToolCallFull::new("read")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file1.txt"}"#));
        let tool_call_2 = ToolCallFull::new("write")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file2.txt"}"#));
        let tool_call_3 = ToolCallFull::new("patch")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file3.txt"}"#));

        // Build history with pattern [1,2,3][1,2] - incomplete repetition
        let msg1 = create_assistant_message(&tool_call_1);
        let msg2 = create_assistant_message(&tool_call_2);
        let msg3 = create_assistant_message(&tool_call_3);
        let msg4 = create_assistant_message(&tool_call_1);
        let msg5 = create_assistant_message(&tool_call_2);

        let conversation = create_conversation_with_messages(vec![msg1, msg2, msg3, msg4, msg5]);

        // Current call would not complete a full third repetition
        let actual = detector.detect_from_conversation(&conversation);

        // Should not detect pattern (incomplete)
        assert_eq!(actual, None);
    }

    #[test]
    fn test_doom_loop_detector_pattern_with_custom_threshold() {
        let detector = DoomLoopDetector::new().threshold(2);

        let tool_call_1 = ToolCallFull::new("read")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file1.txt"}"#));
        let tool_call_2 = ToolCallFull::new("write")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file2.txt"}"#));

        // Build history with pattern [1,2][1,2]
        let msg1 = create_assistant_message(&tool_call_1);
        let msg2 = create_assistant_message(&tool_call_2);
        let msg3 = create_assistant_message(&tool_call_1);
        let msg4 = create_assistant_message(&tool_call_2);

        let conversation = create_conversation_with_messages(vec![msg1, msg2, msg3, msg4]);

        let actual = detector.detect_from_conversation(&conversation);

        // Should detect pattern with threshold of 2
        assert_eq!(actual, Some(2));
    }

    #[test]
    fn test_doom_loop_detector_consecutive_identical_takes_precedence() {
        let detector = DoomLoopDetector::new();

        let tool_call_1 = ToolCallFull::new("read")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file1.txt"}"#));

        // Build history with three consecutive identical calls
        let msg1 = create_assistant_message(&tool_call_1);
        let msg2 = create_assistant_message(&tool_call_1);
        let msg3 = create_assistant_message(&tool_call_1);

        let conversation = create_conversation_with_messages(vec![msg1, msg2, msg3]);

        // Third consecutive identical call - should be caught by consecutive check
        let actual = detector.detect_from_conversation(&conversation);

        assert_eq!(actual, Some(3));
    }

    #[test]
    fn test_doom_loop_detector_complex_pattern_1234_1234_1234() {
        let detector = DoomLoopDetector::new();

        let tool_call_1 = ToolCallFull::new("read")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file1.txt"}"#));
        let tool_call_2 = ToolCallFull::new("write")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file2.txt"}"#));
        let tool_call_3 = ToolCallFull::new("patch")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file3.txt"}"#));
        let tool_call_4 = ToolCallFull::new("shell")
            .arguments(ToolCallArguments::from_json(r#"{"command": "ls"}"#));

        // Build history with pattern [1,2,3,4][1,2,3,4][1,2,3,4]
        let msg1 = create_assistant_message(&tool_call_1);
        let msg2 = create_assistant_message(&tool_call_2);
        let msg3 = create_assistant_message(&tool_call_3);
        let msg4 = create_assistant_message(&tool_call_4);
        let msg5 = create_assistant_message(&tool_call_1);
        let msg6 = create_assistant_message(&tool_call_2);
        let msg7 = create_assistant_message(&tool_call_3);
        let msg8 = create_assistant_message(&tool_call_4);
        let msg9 = create_assistant_message(&tool_call_1);
        let msg10 = create_assistant_message(&tool_call_2);
        let msg11 = create_assistant_message(&tool_call_3);
        let msg12 = create_assistant_message(&tool_call_4);

        let conversation = create_conversation_with_messages(vec![
            msg1, msg2, msg3, msg4, msg5, msg6, msg7, msg8, msg9, msg10, msg11, msg12,
        ]);

        let actual = detector.detect_from_conversation(&conversation);

        // Should detect pattern repetition (3 times)
        assert_eq!(actual, Some(3));
    }

    #[test]
    fn test_doom_loop_detector_real_world_scenario() {
        let detector = DoomLoopDetector::new();

        // Simulate a real-world loop: read file, check diagnostics, patch file
        let read_call = ToolCallFull::new("read")
            .arguments(ToolCallArguments::from_json(r#"{"path": "src/main.rs"}"#));
        let diagnostics_call = ToolCallFull::new("mcp_forge_extension_tool_get_diagnostics")
            .arguments(ToolCallArguments::from_json(r#"{"severity": "error"}"#));
        let patch_call = ToolCallFull::new("patch").arguments(ToolCallArguments::from_json(
            r#"{"path": "src/main.rs", "old": "foo", "new": "bar"}"#,
        ));

        // Create pattern [read, diagnostics, patch] repeated three times
        let msg1 = create_assistant_message(&read_call);
        let msg2 = create_assistant_message(&diagnostics_call);
        let msg3 = create_assistant_message(&patch_call);
        let msg4 = create_assistant_message(&read_call);
        let msg5 = create_assistant_message(&diagnostics_call);
        let msg6 = create_assistant_message(&patch_call);
        let msg7 = create_assistant_message(&read_call);
        let msg8 = create_assistant_message(&diagnostics_call);
        let msg9 = create_assistant_message(&patch_call);

        let conversation = create_conversation_with_messages(vec![
            msg1, msg2, msg3, msg4, msg5, msg6, msg7, msg8, msg9,
        ]);

        let actual = detector.detect_from_conversation(&conversation);

        // Should detect the pattern loop
        assert_eq!(actual, Some(3));
    }

    #[test]
    fn test_doom_loop_detector_pattern_changes_midway_123123454545() {
        let detector = DoomLoopDetector::new();

        let tool_call_1 = ToolCallFull::new("read")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file1.txt"}"#));
        let tool_call_2 = ToolCallFull::new("write")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file2.txt"}"#));
        let tool_call_3 = ToolCallFull::new("patch")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file3.txt"}"#));
        let tool_call_4 = ToolCallFull::new("shell")
            .arguments(ToolCallArguments::from_json(r#"{"command": "ls"}"#));
        let tool_call_5 = ToolCallFull::new("fs_search")
            .arguments(ToolCallArguments::from_json(r#"{"pattern": "test"}"#));

        // Build history with pattern [1,2,3][1,2,3] then [4,5][4,5][4,5]
        // Pattern: 123123454545
        let msg1 = create_assistant_message(&tool_call_1);
        let msg2 = create_assistant_message(&tool_call_2);
        let msg3 = create_assistant_message(&tool_call_3);
        let msg4 = create_assistant_message(&tool_call_1);
        let msg5 = create_assistant_message(&tool_call_2);
        let msg6 = create_assistant_message(&tool_call_3);
        let msg7 = create_assistant_message(&tool_call_4);
        let msg8 = create_assistant_message(&tool_call_5);
        let msg9 = create_assistant_message(&tool_call_4);
        let msg10 = create_assistant_message(&tool_call_5);
        let msg11 = create_assistant_message(&tool_call_4);
        let msg12 = create_assistant_message(&tool_call_5);

        let conversation = create_conversation_with_messages(vec![
            msg1, msg2, msg3, msg4, msg5, msg6, msg7, msg8, msg9, msg10, msg11, msg12,
        ]);

        // [4,5][4,5][4,5] now fully exists at the end
        let actual = detector.detect_from_conversation(&conversation);

        // Should detect the [4,5][4,5][4,5] pattern at the end
        // The detector looks for the longest repeating pattern, starting from the most
        // recent calls
        // The pattern [4,5] repeats 3 times at the end
        assert_eq!(actual, Some(3));
    }

    #[test]
    fn test_doom_loop_detector_sequence_1234546454545_step_by_step() {
        let detector = DoomLoopDetector::new();

        // Define the 6 unique tool calls
        let tool_1 = ToolCallFull::new("read")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file1.txt"}"#));
        let tool_2 = ToolCallFull::new("write")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file2.txt"}"#));
        let tool_3 = ToolCallFull::new("patch")
            .arguments(ToolCallArguments::from_json(r#"{"path": "file3.txt"}"#));
        let tool_4 = ToolCallFull::new("shell")
            .arguments(ToolCallArguments::from_json(r#"{"command": "ls"}"#));
        let tool_5 = ToolCallFull::new("fs_search")
            .arguments(ToolCallArguments::from_json(r#"{"pattern": "test"}"#));
        let tool_6 = ToolCallFull::new("sem_search")
            .arguments(ToolCallArguments::from_json(r#"{"queries": []}"#));

        // Sequence: 1234546454545
        // Let's build it step by step and check at each step
        let mut messages = vec![];

        // Step 1: [1] - no loop
        messages.push(create_assistant_message(&tool_1));
        let conv = create_conversation_with_messages(messages.clone());
        assert_eq!(detector.detect_from_conversation(&conv), None);

        // Step 2: [1,2] - no loop
        messages.push(create_assistant_message(&tool_2));
        let conv = create_conversation_with_messages(messages.clone());
        assert_eq!(detector.detect_from_conversation(&conv), None);

        // Step 3: [1,2,3] - no loop
        messages.push(create_assistant_message(&tool_3));
        let conv = create_conversation_with_messages(messages.clone());
        assert_eq!(detector.detect_from_conversation(&conv), None);

        // Step 4: [1,2,3,4] - no loop
        messages.push(create_assistant_message(&tool_4));
        let conv = create_conversation_with_messages(messages.clone());
        assert_eq!(detector.detect_from_conversation(&conv), None);

        // Step 5: [1,2,3,4,5] - no loop
        messages.push(create_assistant_message(&tool_5));
        let conv = create_conversation_with_messages(messages.clone());
        assert_eq!(detector.detect_from_conversation(&conv), None);

        // Step 6: [1,2,3,4,5,4] - no loop yet
        messages.push(create_assistant_message(&tool_4));
        let conv = create_conversation_with_messages(messages.clone());
        assert_eq!(detector.detect_from_conversation(&conv), None);

        // Step 7: [1,2,3,4,5,4,6] - no loop
        messages.push(create_assistant_message(&tool_6));
        let conv = create_conversation_with_messages(messages.clone());
        assert_eq!(detector.detect_from_conversation(&conv), None);

        // Step 8: [1,2,3,4,5,4,6,4] - no loop yet
        messages.push(create_assistant_message(&tool_4));
        let conv = create_conversation_with_messages(messages.clone());
        assert_eq!(detector.detect_from_conversation(&conv), None);

        // Step 9: [1,2,3,4,5,4,6,4,5] - no loop yet (only 1.5 repetitions of [4,5])
        messages.push(create_assistant_message(&tool_5));
        let conv = create_conversation_with_messages(messages.clone());
        assert_eq!(detector.detect_from_conversation(&conv), None);

        // Step 10: [1,2,3,4,5,4,6,4,5,4] - no loop yet (2 repetitions of [4,5])
        messages.push(create_assistant_message(&tool_4));
        let conv = create_conversation_with_messages(messages.clone());
        assert_eq!(detector.detect_from_conversation(&conv), None);

        // Step 11: [1,2,3,4,5,4,6,4,5,4,5] - still no loop (2.5 repetitions)
        messages.push(create_assistant_message(&tool_5));
        let conv = create_conversation_with_messages(messages.clone());
        assert_eq!(detector.detect_from_conversation(&conv), None);

        // Step 12: [1,2,3,4,5,4,6,4,5,4,5,4] - still no loop (almost 3)
        messages.push(create_assistant_message(&tool_4));
        let conv = create_conversation_with_messages(messages.clone());
        assert_eq!(detector.detect_from_conversation(&conv), None);

        // Step 13: [1,2,3,4,5,4,6,4,5,4,5,4,5] - [4,5] pattern now repeats 3 times at
        // end
        messages.push(create_assistant_message(&tool_5));
        let conv = create_conversation_with_messages(messages.clone());

        let result = detector.detect_from_conversation(&conv);

        // Should detect pattern [4,5] repeating 3 times
        assert_eq!(result, Some(3));
    }
}
