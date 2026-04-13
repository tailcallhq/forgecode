use forge_domain::{ContextSummary, Role, SummaryMessage, SummaryTool, Transformer};

/// Removes redundant operations from the context summary.
///
/// This transformer deduplicates consecutive operations within assistant
/// messages by retaining only the most recent operation for each resource
/// (e.g., file path, command). Only applies to messages with the Assistant
/// role. This is useful for reducing context size while preserving the final
/// state of operations.
pub struct TrimContextSummary;

/// Represents the type and target of a tool call operation.
///
/// Used for identifying and comparing operations to determine if they operate
/// on the same resource (e.g., same file path, same shell command).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Operation<'a> {
    /// File operation (read, update, remove, undo) on a specific path
    File(&'a str),
    /// Shell command execution
    Shell(&'a str),
    /// Search operation with a specific pattern
    Search(&'a str),
    /// Codebase search operation with queries
    CodebaseSearch {
        queries: &'a [forge_domain::SearchQuery],
    },
    /// Fetch operation for a specific URL
    Fetch(&'a str),
    /// Follow-up question
    Followup(&'a str),
    /// Plan creation with a specific name
    Plan(&'a str),
    /// Skill loading by name
    Skill(&'a str),
    /// Task delegation to an agent
    Task(&'a str),
    /// MCP tool call by name
    Mcp(&'a str),
    /// Todo operation - each todo_write is unique and won't be deduplicated
    Todo,
}

/// Converts the tool call to its operation type for comparison.
///
/// File operations (read, update, remove, undo) on the same path are
/// considered the same operation type for deduplication purposes.
fn to_op(tool: &SummaryTool) -> Operation<'_> {
    match tool {
        SummaryTool::FileRead { path } => Operation::File(path),
        SummaryTool::FileUpdate { path } => Operation::File(path),
        SummaryTool::FileRemove { path } => Operation::File(path),
        SummaryTool::Undo { path } => Operation::File(path),
        SummaryTool::Shell { command } => Operation::Shell(command),
        SummaryTool::Search { pattern } => Operation::Search(pattern),
        SummaryTool::SemSearch { queries } => Operation::CodebaseSearch { queries },
        SummaryTool::Fetch { url } => Operation::Fetch(url),
        SummaryTool::Followup { question } => Operation::Followup(question),
        SummaryTool::Plan { plan_name } => Operation::Plan(plan_name),
        SummaryTool::Skill { name } => Operation::Skill(name),
        SummaryTool::Task { agent_id } => Operation::Task(agent_id),
        SummaryTool::Mcp { name } => Operation::Mcp(name),
        SummaryTool::TodoWrite { .. } => Operation::Todo,
        SummaryTool::TodoRead => Operation::Todo,
    }
}

impl Transformer for TrimContextSummary {
    type Value = ContextSummary;

    fn transform(&mut self, mut summary: Self::Value) -> Self::Value {
        for message in summary.messages.iter_mut() {
            // Only apply trimming to Assistant role messages
            if message.role != Role::Assistant {
                continue;
            }

            let mut block_seq: Vec<SummaryMessage> = Default::default();

            for block in message.contents.drain(..) {
                // For tool calls, only keep successful operations
                if let SummaryMessage::ToolCall(ref tool_call) = block {
                    // Remove previous entry if it has the same operation
                    if let Some(SummaryMessage::ToolCall(last_tool_call)) = block_seq.last_mut()
                        && to_op(&last_tool_call.tool) == to_op(&tool_call.tool)
                    {
                        block_seq.pop();
                    }
                }

                block_seq.push(block);
            }

            message.contents = block_seq;
        }

        summary
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Role, SummaryBlock, SummaryToolCall, ToolCallId};
    use pretty_assertions::assert_eq;

    use super::*;

    // Alias for convenience in tests
    type Block = SummaryMessage;

    #[test]
    fn test_empty_summary() {
        let fixture = ContextSummary::new(vec![]);
        let actual = TrimContextSummary.transform(fixture);

        let expected = ContextSummary::new(vec![]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_keeps_last_operation_per_path() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("/test1").into(),
                SummaryToolCall::read("/test2").into(),
                SummaryToolCall::read("/test2").into(),
                SummaryToolCall::read("/test3").into(),
            ],
        )]);
        let actual = TrimContextSummary.transform(fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("/test1").into(),
                SummaryToolCall::read("/test2").into(),
                SummaryToolCall::read("/test3").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_keeps_last_operation_with_content() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("/test")
                    .id(ToolCallId::new("call1"))
                    .into(),
                SummaryToolCall::read("/test")
                    .id(ToolCallId::new("call2"))
                    .into(),
            ],
        )]);
        let actual = TrimContextSummary.transform(fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("/test")
                    .id(ToolCallId::new("call2"))
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_different_operation_types_on_same_path() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("/test").into(),
                SummaryToolCall::read("/test").into(),
                SummaryToolCall::update("file.txt").into(),
                SummaryToolCall::update("file.txt").into(),
                SummaryToolCall::read("/test").into(),
                SummaryToolCall::update("/test").into(),
                SummaryToolCall::remove("/test").into(),
            ],
        )]);
        let actual = TrimContextSummary.transform(fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("/test").into(),
                SummaryToolCall::update("file.txt").into(),
                SummaryToolCall::remove("/test").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_filters_failed_and_none_operations() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("/test").into(),
                SummaryToolCall::read("/test").is_success(false).into(),
                SummaryToolCall::read("/test").into(),
                SummaryToolCall::read("/unknown").into(),
                SummaryToolCall::read("/unknown").is_success(false).into(),
                SummaryToolCall::update("file.txt").into(),
                SummaryToolCall::read("/all_failed")
                    .is_success(false)
                    .into(),
            ],
        )]);
        let actual = TrimContextSummary.transform(fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("/test").into(),
                SummaryToolCall::read("/unknown").is_success(false).into(),
                SummaryToolCall::update("file.txt").into(),
                SummaryToolCall::read("/all_failed")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_only_trims_assistant_messages() {
        let fixture = ContextSummary::new(vec![
            SummaryBlock::new(
                Role::User,
                vec![
                    SummaryToolCall::read("/test").into(),
                    SummaryToolCall::read("/test").into(),
                ],
            ),
            SummaryBlock::new(
                Role::Assistant,
                vec![
                    SummaryToolCall::update("file.txt").into(),
                    SummaryToolCall::update("file.txt").into(),
                ],
            ),
            SummaryBlock::new(
                Role::System,
                vec![
                    SummaryToolCall::remove("remove.txt").into(),
                    SummaryToolCall::remove("remove.txt").into(),
                ],
            ),
            SummaryBlock::new(
                Role::Assistant,
                vec![
                    SummaryToolCall::read("/test").into(),
                    SummaryToolCall::read("/test").into(),
                ],
            ),
        ]);
        let actual = TrimContextSummary.transform(fixture);

        let expected = ContextSummary::new(vec![
            SummaryBlock::new(
                Role::User,
                vec![
                    SummaryToolCall::read("/test").into(),
                    SummaryToolCall::read("/test").into(),
                ],
            ),
            SummaryBlock::new(
                Role::Assistant,
                vec![SummaryToolCall::update("file.txt").into()],
            ),
            SummaryBlock::new(
                Role::System,
                vec![
                    SummaryToolCall::remove("remove.txt").into(),
                    SummaryToolCall::remove("remove.txt").into(),
                ],
            ),
            SummaryBlock::new(Role::Assistant, vec![SummaryToolCall::read("/test").into()]),
        ]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_multiple_assistant_messages_trimmed_independently() {
        let fixture = ContextSummary::new(vec![
            SummaryBlock::new(
                Role::Assistant,
                vec![
                    SummaryToolCall::read("/test").into(),
                    SummaryToolCall::read("/test").into(),
                ],
            ),
            SummaryBlock::new(
                Role::Assistant,
                vec![SummaryToolCall::read("/test").is_success(false).into()],
            ),
            SummaryBlock::new(
                Role::Assistant,
                vec![
                    SummaryToolCall::read("/test").into(),
                    SummaryToolCall::read("/test").into(),
                    SummaryToolCall::read("/test").into(),
                ],
            ),
        ]);
        let actual = TrimContextSummary.transform(fixture);

        let expected = ContextSummary::new(vec![
            SummaryBlock::new(Role::Assistant, vec![SummaryToolCall::read("/test").into()]),
            SummaryBlock::new(
                Role::Assistant,
                vec![SummaryToolCall::read("/test").is_success(false).into()],
            ),
            SummaryBlock::new(Role::Assistant, vec![SummaryToolCall::read("/test").into()]),
        ]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_assistant_message_with_different_call_ids() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("foo"),
                SummaryToolCall::read("/test1")
                    .id(ToolCallId::new("1"))
                    .into(),
                SummaryToolCall::read("/test1")
                    .id(ToolCallId::new("2"))
                    .into(),
            ],
        )]);
        let actual = TrimContextSummary.transform(fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("foo"),
                SummaryToolCall::read("/test1")
                    .id(ToolCallId::new("2"))
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_preserves_shell_commands() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::shell("cargo build").into(),
                SummaryToolCall::shell("cargo test").into(),
                SummaryToolCall::shell("cargo build").into(),
            ],
        )]);
        let actual = TrimContextSummary.transform(fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::shell("cargo build").into(),
                SummaryToolCall::shell("cargo test").into(),
                SummaryToolCall::shell("cargo build").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_mixed_shell_and_file_operations() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("/test.rs").into(),
                SummaryToolCall::shell("cargo build").into(),
                SummaryToolCall::read("/test.rs").into(),
                SummaryToolCall::shell("cargo test").into(),
                SummaryToolCall::update("/output.txt").into(),
            ],
        )]);
        let actual = TrimContextSummary.transform(fixture);

        // Shell commands break the deduplication chain, so both reads of /test.rs are
        // preserved
        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("/test.rs").into(),
                SummaryToolCall::shell("cargo build").into(),
                SummaryToolCall::read("/test.rs").into(),
                SummaryToolCall::shell("cargo test").into(),
                SummaryToolCall::update("/output.txt").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_shell_commands_between_file_operations_on_same_path() {
        let fixture = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("/test.rs").into(),
                SummaryToolCall::shell("cargo build").into(),
                SummaryToolCall::read("/test.rs").into(),
                SummaryToolCall::shell("cargo test").into(),
                SummaryToolCall::read("/test.rs").into(),
            ],
        )]);
        let actual = TrimContextSummary.transform(fixture);

        // Shell commands break the deduplication chain - all reads are preserved
        // because shell commands are interspersed between them
        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                SummaryToolCall::read("/test.rs").into(),
                SummaryToolCall::shell("cargo build").into(),
                SummaryToolCall::read("/test.rs").into(),
                SummaryToolCall::shell("cargo test").into(),
                SummaryToolCall::read("/test.rs").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }
}
