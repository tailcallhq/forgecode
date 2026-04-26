use std::collections::HashMap;
use std::ops::Deref;

use derive_more::From;
use serde::{Deserialize, Serialize};

use crate::{
    Context, ContextMessage, Role, SearchQuery, TextMessage, Todo, ToolCallFull, ToolCallId,
    ToolCatalog, ToolResult,
};

/// A simplified summary of a context, focusing on messages and their tool calls
#[derive(Default, PartialEq, Debug, Serialize, Deserialize, derive_setters::Setters)]
#[setters(strip_option)]
#[serde(rename_all = "snake_case")]
pub struct ContextSummary {
    pub messages: Vec<SummaryBlock>,
}

/// A simplified representation of a message with its key information
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, derive_setters::Setters)]
#[setters(strip_option)]
#[serde(rename_all = "snake_case")]
pub struct SummaryBlock {
    pub role: Role,
    pub contents: Vec<SummaryMessage>,
}

/// A message block that can be either content or a tool call
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, From)]
#[serde(rename_all = "snake_case")]
pub enum SummaryMessage {
    Text(String),
    ToolCall(#[from] SummaryToolCall),
}

/// Tool call data with execution status
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, derive_setters::Setters)]
#[setters(strip_option, into)]
#[serde(rename_all = "snake_case")]
pub struct SummaryToolCall {
    pub id: Option<ToolCallId>,
    pub tool: SummaryTool,
    pub is_success: bool,
}

impl ContextSummary {
    /// Creates a new ContextSummary with the given messages
    pub fn new(messages: Vec<SummaryBlock>) -> Self {
        Self { messages }
    }
}

impl SummaryBlock {
    /// Creates a new SummaryMessage with the given role and blocks
    pub fn new(role: Role, blocks: Vec<SummaryMessage>) -> Self {
        Self { role, contents: blocks }
    }
}

impl SummaryMessage {
    /// Creates a content block
    pub fn content(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }
}

impl SummaryToolCall {
    /// Creates a FileRead tool call with default values (id: None, is_success:
    /// true)
    pub fn read(path: impl Into<String>) -> Self {
        Self {
            id: None,
            tool: SummaryTool::FileRead { path: path.into() },
            is_success: true,
        }
    }

    /// Creates a FileUpdate tool call with default values (id: None,
    /// is_success: true)
    pub fn update(path: impl Into<String>) -> Self {
        Self {
            id: None,
            tool: SummaryTool::FileUpdate { path: path.into() },
            is_success: true,
        }
    }

    /// Creates a FileRemove tool call with default values (id: None,
    /// is_success: true)
    pub fn remove(path: impl Into<String>) -> Self {
        Self {
            id: None,
            tool: SummaryTool::FileRemove { path: path.into() },
            is_success: true,
        }
    }

    /// Creates a Shell tool call with default values (id: None, is_success:
    /// true)
    pub fn shell(command: impl Into<String>) -> Self {
        Self {
            id: None,
            tool: SummaryTool::Shell { command: command.into() },
            is_success: true,
        }
    }

    /// Creates a Search tool call with default values (id: None, is_success:
    /// true)
    pub fn search(pattern: impl Into<String>) -> Self {
        Self {
            id: None,
            tool: SummaryTool::Search { pattern: pattern.into() },
            is_success: true,
        }
    }

    /// Creates a CodebaseSearch tool call with default values (id: None,
    /// is_success: true)
    pub fn codebase_search(queries: Vec<SearchQuery>) -> Self {
        Self {
            id: None,
            tool: SummaryTool::SemSearch { queries },
            is_success: true,
        }
    }

    /// Creates an Undo tool call with default values (id: None, is_success:
    /// true)
    pub fn undo(path: impl Into<String>) -> Self {
        Self {
            id: None,
            tool: SummaryTool::Undo { path: path.into() },
            is_success: true,
        }
    }

    /// Creates a Fetch tool call with default values (id: None, is_success:
    /// true)
    pub fn fetch(url: impl Into<String>) -> Self {
        Self {
            id: None,
            tool: SummaryTool::Fetch { url: url.into() },
            is_success: true,
        }
    }

    /// Creates a Followup tool call with default values (id: None, is_success:
    /// true)
    pub fn followup(question: impl Into<String>) -> Self {
        Self {
            id: None,
            tool: SummaryTool::Followup { question: question.into() },
            is_success: true,
        }
    }

    /// Creates a Plan tool call with default values (id: None, is_success:
    /// true)
    pub fn plan(plan_name: impl Into<String>) -> Self {
        Self {
            id: None,
            tool: SummaryTool::Plan { plan_name: plan_name.into() },
            is_success: true,
        }
    }

    /// Creates an MCP tool call with default values (id: None, is_success:
    /// true)
    pub fn mcp(name: impl Into<String>) -> Self {
        Self {
            id: None,
            tool: SummaryTool::Mcp { name: name.into() },
            is_success: true,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryTool {
    FileRead { path: String },
    FileUpdate { path: String },
    FileRemove { path: String },
    Shell { command: String },
    Search { pattern: String },
    SemSearch { queries: Vec<SearchQuery> },
    Undo { path: String },
    Fetch { url: String },
    Followup { question: String },
    Plan { plan_name: String },
    Skill { name: String },
    Task { agent_id: String },
    Mcp { name: String },
    TodoWrite { changes: Vec<TodoChange> },
    TodoRead,
}

/// The kind of change applied to a todo item
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoChangeKind {
    Added,
    Updated,
    Removed,
}

/// A single todo change entry capturing what changed and how
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TodoChange {
    pub todo: Todo,
    pub kind: TodoChangeKind,
}

impl From<&Context> for ContextSummary {
    fn from(value: &Context) -> Self {
        let mut messages = vec![];
        let mut buffer: Vec<SummaryMessage> = vec![];
        let mut tool_results: HashMap<&ToolCallId, &ToolResult> = Default::default();
        let mut current_role = Role::System;
        // Track the current todo state to compute diffs across tool calls
        let mut current_todos: Vec<Todo> = vec![];
        for msg in &value.messages {
            match msg.deref() {
                ContextMessage::Text(text_msg) => {
                    // Skip system messages
                    if text_msg.role == Role::System {
                        continue;
                    }

                    if current_role != text_msg.role {
                        // Only push if buffer is not empty (avoid empty System role at start)
                        if !buffer.is_empty() {
                            messages.push(SummaryBlock {
                                role: current_role,
                                contents: std::mem::take(&mut buffer),
                            });
                        }

                        current_role = text_msg.role;
                    }

                    buffer.extend(extract_summary_messages(text_msg, &current_todos));

                    // Update current_todos if this is a TodoWrite call
                    if let Some(calls) = &text_msg.tool_calls {
                        for call in calls {
                            if let Ok(ToolCatalog::TodoWrite(input)) =
                                ToolCatalog::try_from(call.clone())
                            {
                                for item in &input.todos {
                                    if item.status == crate::TodoStatus::Cancelled {
                                        current_todos.retain(|t| t.content != item.content);
                                    } else if let Some(existing) =
                                        current_todos.iter_mut().find(|t| t.content == item.content)
                                    {
                                        existing.status = item.status;
                                    } else {
                                        current_todos.push(Todo {
                                            id: String::new(),
                                            content: item.content.clone(),
                                            status: item.status,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                ContextMessage::Tool(tool_result) => {
                    if let Some(ref call_id) = tool_result.call_id {
                        tool_results.insert(call_id, tool_result);
                    }
                }
                ContextMessage::Image(_) => {}
            }
        }

        // Insert the last chunk if buffer is not empty
        if !buffer.is_empty() {
            messages
                .push(SummaryBlock { role: current_role, contents: std::mem::take(&mut buffer) });
        }

        // Update tool call success status based on results
        messages
            .iter_mut()
            .flat_map(|message| message.contents.iter_mut())
            .for_each(|block| {
                if let SummaryMessage::ToolCall(tool_data) = block
                    && let Some(call_id) = &tool_data.id
                    && let Some(result) = tool_results.get(call_id)
                {
                    tool_data.is_success = !result.is_error();
                }
            });

        ContextSummary { messages }
    }
}

/// Extracts summary messages from a text message, using current_todos for diff
/// computation
fn extract_summary_messages(text_msg: &TextMessage, current_todos: &[Todo]) -> Vec<SummaryMessage> {
    let mut blocks = vec![];

    // Add content block if there's text content
    if !text_msg.content.is_empty() {
        blocks.push(SummaryMessage::Text(text_msg.content.clone()));
    }

    // Add tool call blocks if present
    if let Some(calls) = &text_msg.tool_calls {
        blocks.extend(calls.iter().filter_map(|tool_call| {
            extract_tool_info(tool_call, current_todos).map(|call| {
                SummaryMessage::ToolCall(SummaryToolCall {
                    id: tool_call.call_id.clone(),
                    tool: call,
                    is_success: false,
                })
            })
        }));
    }

    blocks
}

impl From<&TextMessage> for Vec<SummaryMessage> {
    fn from(text_msg: &TextMessage) -> Self {
        extract_summary_messages(text_msg, &[])
    }
}

/// Extracts tool information from a tool call, using current_todos as the
/// before-state for diffs
fn extract_tool_info(call: &ToolCallFull, current_todos: &[Todo]) -> Option<SummaryTool> {
    // Try to parse as a Tools enum variant
    if let Ok(tool) = ToolCatalog::try_from(call.clone()) {
        return match tool {
            ToolCatalog::Read(input) => Some(SummaryTool::FileRead { path: input.file_path }),
            ToolCatalog::Write(input) => Some(SummaryTool::FileUpdate { path: input.file_path }),
            ToolCatalog::Patch(input) => Some(SummaryTool::FileUpdate { path: input.file_path }),
            ToolCatalog::MultiPatch(input) => {
                Some(SummaryTool::FileUpdate { path: input.file_path })
            }
            ToolCatalog::Remove(input) => Some(SummaryTool::FileRemove { path: input.path }),
            ToolCatalog::Shell(input) => Some(SummaryTool::Shell { command: input.command }),
            ToolCatalog::FsSearch(input) => {
                // Use glob, file_type, or pattern as the search identifier
                let pattern = input.glob.or(input.file_type).unwrap_or(input.pattern);
                Some(SummaryTool::Search { pattern })
            }
            ToolCatalog::SemSearch(input) => {
                Some(SummaryTool::SemSearch { queries: input.queries })
            }
            ToolCatalog::Undo(input) => Some(SummaryTool::Undo { path: input.path }),
            ToolCatalog::Fetch(input) => Some(SummaryTool::Fetch { url: input.url }),
            ToolCatalog::Followup(input) => {
                Some(SummaryTool::Followup { question: input.question })
            }
            ToolCatalog::Plan(input) => Some(SummaryTool::Plan { plan_name: input.plan_name }),
            ToolCatalog::Skill(input) => Some(SummaryTool::Skill { name: input.name }),
            ToolCatalog::TodoWrite(input) => {
                let before_map: HashMap<&str, &Todo> = current_todos
                    .iter()
                    .map(|t| (t.content.as_str(), t))
                    .collect();

                let mut changes = vec![];

                for item in &input.todos {
                    if item.status == crate::TodoStatus::Cancelled {
                        if let Some(prev) = before_map.get(item.content.as_str()) {
                            changes.push(TodoChange {
                                todo: (*prev).clone(),
                                kind: TodoChangeKind::Removed,
                            });
                        }
                    } else {
                        match before_map.get(item.content.as_str()) {
                            None => changes.push(TodoChange {
                                todo: Todo {
                                    id: String::new(),
                                    content: item.content.clone(),
                                    status: item.status,
                                },
                                kind: TodoChangeKind::Added,
                            }),
                            Some(prev) if prev.status != item.status => {
                                changes.push(TodoChange {
                                    todo: Todo {
                                        id: prev.id.clone(),
                                        content: item.content.clone(),
                                        status: item.status,
                                    },
                                    kind: TodoChangeKind::Updated,
                                });
                            }
                            _ => {}
                        }
                    }
                }

                Some(SummaryTool::TodoWrite { changes })
            }
            ToolCatalog::TodoRead(_) => Some(SummaryTool::TodoRead),
            ToolCatalog::Task(input) => Some(SummaryTool::Task { agent_id: input.agent_id }),
        };
    }

    // If not a known tool catalog item, treat as MCP tool
    Some(SummaryTool::Mcp { name: call.name.to_string() })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::{ContextMessage, TextMessage, ToolCallArguments, ToolCallId, ToolName, ToolOutput};

    type Block = SummaryMessage;

    fn context(messages: Vec<ContextMessage>) -> Context {
        Context::default().messages(messages.into_iter().map(|m| m.into()).collect::<Vec<_>>())
    }

    fn user(content: impl Into<String>) -> ContextMessage {
        ContextMessage::Text(TextMessage::new(Role::User, content))
    }

    fn assistant(content: impl Into<String>) -> ContextMessage {
        ContextMessage::Text(TextMessage::new(Role::Assistant, content))
    }

    fn assistant_with_tools(
        content: impl Into<String>,
        tool_calls: Vec<ToolCallFull>,
    ) -> ContextMessage {
        ContextMessage::Text(TextMessage::new(Role::Assistant, content).tool_calls(tool_calls))
    }

    fn system(content: impl Into<String>) -> ContextMessage {
        ContextMessage::Text(TextMessage::new(Role::System, content))
    }

    fn tool_result(name: &str, call_id: &str, is_error: bool) -> ContextMessage {
        ContextMessage::Tool(ToolResult {
            name: ToolName::new(name),
            call_id: Some(ToolCallId::new(call_id)),
            output: ToolOutput::text("result").is_error(is_error),
        })
    }

    #[test]
    fn test_summary_message_block_read_helper() {
        let actual: SummaryMessage = SummaryToolCall::read("/path/to/file.rs").into();

        let expected = Block::ToolCall(SummaryToolCall {
            id: None,
            tool: SummaryTool::FileRead { path: "/path/to/file.rs".to_string() },
            is_success: true,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_summary_message_block_update_helper() {
        let actual: SummaryMessage = SummaryToolCall::update("/path/to/file.rs").into();

        let expected = Block::ToolCall(SummaryToolCall {
            id: None,
            tool: SummaryTool::FileUpdate { path: "/path/to/file.rs".to_string() },
            is_success: true,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_summary_message_block_remove_helper() {
        let actual: SummaryMessage = SummaryToolCall::remove("/path/to/file.rs").into();

        let expected = Block::ToolCall(SummaryToolCall {
            id: None,
            tool: SummaryTool::FileRemove { path: "/path/to/file.rs".to_string() },
            is_success: true,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_empty_context() {
        let fixture = Context::default();
        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::default();

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_user_and_assistant_without_tools() {
        let fixture = context(vec![
            user("Please help me"),
            assistant("Sure, I can help"),
            user("Thanks"),
            assistant("You're welcome"),
        ]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![
            SummaryBlock::new(Role::User, vec![Block::content("Please help me")]),
            SummaryBlock::new(Role::Assistant, vec![Block::content("Sure, I can help")]),
            SummaryBlock::new(Role::User, vec![Block::content("Thanks")]),
            SummaryBlock::new(Role::Assistant, vec![Block::content("You're welcome")]),
        ]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_skips_system_messages() {
        let fixture = context(vec![system("System prompt"), user("User message")]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::User,
            vec![Block::content("User message")],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_extracts_file_read_tool_calls() {
        let fixture = context(vec![assistant_with_tools(
            "Reading file",
            vec![ToolCatalog::tool_call_read("/test/file.rs").call_id("call_1")],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Reading file"),
                SummaryToolCall::read("/test/file.rs")
                    .id("call_1")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_extracts_file_write_tool_calls() {
        let fixture = context(vec![assistant_with_tools(
            "Writing file",
            vec![ToolCatalog::tool_call_write("/test/file.rs", "test").call_id("call_1")],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Writing file"),
                SummaryToolCall::update("/test/file.rs")
                    .id("call_1")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_extracts_file_patch_tool_calls() {
        let fixture = context(vec![assistant_with_tools(
            "Patching file",
            vec![
                ToolCatalog::tool_call_patch("/test/file.rs", "new", "old", false)
                    .call_id("call_1"),
            ],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Patching file"),
                SummaryToolCall::update("/test/file.rs")
                    .id("call_1")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_extracts_file_remove_tool_calls() {
        let fixture = context(vec![assistant_with_tools(
            "Removing file",
            vec![ToolCatalog::tool_call_remove("/test/file.rs").call_id("call_1")],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Removing file"),
                SummaryToolCall::remove("/test/file.rs")
                    .id("call_1")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_extracts_read_image_tool_calls() {
        let fixture = context(vec![assistant_with_tools(
            "Reading image",
            vec![ToolCatalog::tool_call_read("/test/image.png").call_id("call_1")],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Reading image"),
                SummaryToolCall::read("/test/image.png")
                    .id("call_1")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_extracts_shell_tool_calls() {
        let fixture = context(vec![assistant_with_tools(
            "Running shell",
            vec![ToolCatalog::tool_call_shell("ls -la", "/test").call_id("call_1")],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Running shell"),
                SummaryToolCall::shell("ls -la")
                    .id("call_1")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_multiple_tool_calls_in_message() {
        let fixture = context(vec![assistant_with_tools(
            "Multiple operations",
            vec![
                ToolCatalog::tool_call_read("/test/file1.rs").call_id("call_1"),
                ToolCatalog::tool_call_write("/test/file2.rs", "test").call_id("call_2"),
                ToolCatalog::tool_call_remove("/test/file3.rs").call_id("call_3"),
            ],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Multiple operations"),
                SummaryToolCall::read("/test/file1.rs")
                    .id("call_1")
                    .is_success(false)
                    .into(),
                SummaryToolCall::update("/test/file2.rs")
                    .id("call_2")
                    .is_success(false)
                    .into(),
                SummaryToolCall::remove("/test/file3.rs")
                    .id("call_3")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_links_tool_results_to_calls_success() {
        let fixture = context(vec![
            assistant_with_tools(
                "Reading file",
                vec![ToolCatalog::tool_call_read("/test/file.rs").call_id("call_1")],
            ),
            tool_result("read", "call_1", false),
        ]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Reading file"),
                SummaryToolCall::read("/test/file.rs").id("call_1").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_links_tool_results_to_calls_failure() {
        let fixture = context(vec![
            assistant_with_tools(
                "Reading file",
                vec![ToolCatalog::tool_call_read("/test/file.rs").call_id("call_1")],
            ),
            tool_result("read", "call_1", true),
        ]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Reading file"),
                SummaryToolCall::read("/test/file.rs")
                    .id("call_1")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_multiple_tool_results() {
        let fixture = context(vec![
            assistant_with_tools(
                "Multiple operations",
                vec![
                    ToolCatalog::tool_call_read("/test/file1.rs").call_id("call_1"),
                    ToolCatalog::tool_call_write("/test/file2.rs", "test").call_id("call_2"),
                ],
            ),
            tool_result("read", "call_1", false),
            tool_result("write", "call_2", true),
        ]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Multiple operations"),
                SummaryToolCall::read("/test/file1.rs").id("call_1").into(),
                SummaryToolCall::update("/test/file2.rs")
                    .id("call_2")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_tool_result_without_call_id() {
        let fixture = context(vec![
            assistant_with_tools(
                "Reading file",
                vec![ToolCatalog::tool_call_read("/test/file.rs").call_id("call_1")],
            ),
            ContextMessage::Tool(ToolResult {
                name: ToolName::new("read"),
                call_id: None,
                output: ToolOutput::text("result"),
            }),
        ]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Reading file"),
                SummaryToolCall::read("/test/file.rs")
                    .id("call_1")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_complex_conversation() {
        let fixture = context(vec![
            system("System prompt"),
            user("Read this file"),
            assistant_with_tools(
                "Reading",
                vec![ToolCatalog::tool_call_read("/test/file1.rs").call_id("call_1")],
            ),
            tool_result("read", "call_1", false),
            user("Now update it"),
            assistant_with_tools(
                "Updating",
                vec![
                    ToolCatalog::tool_call_write("/test/file1.rs", "new content").call_id("call_2"),
                ],
            ),
            tool_result("write", "call_2", false),
            assistant("Done"),
        ]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![
            SummaryBlock::new(Role::User, vec![Block::content("Read this file")]),
            SummaryBlock::new(
                Role::Assistant,
                vec![
                    Block::content("Reading"),
                    SummaryToolCall::read("/test/file1.rs").id("call_1").into(),
                ],
            ),
            SummaryBlock::new(Role::User, vec![Block::content("Now update it")]),
            SummaryBlock::new(
                Role::Assistant,
                vec![
                    Block::content("Updating"),
                    SummaryToolCall::update("/test/file1.rs")
                        .id("call_2")
                        .into(),
                    Block::content("Done"),
                ],
            ),
        ]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_ignores_image_messages() {
        let fixture = context(vec![
            user("User message"),
            ContextMessage::Image(crate::Image::new_base64(
                "test_image_data".to_string(),
                "image/png",
            )),
            assistant("Assistant"),
        ]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![
            SummaryBlock::new(Role::User, vec![Block::content("User message")]),
            SummaryBlock::new(Role::Assistant, vec![Block::content("Assistant")]),
        ]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_extract_tool_info_with_mcp_tool() {
        let fixture = ToolCallFull {
            name: ToolName::new("mcp_github_create_issue"),
            call_id: Some(ToolCallId::new("call_1")),
            arguments: ToolCallArguments::from_json(r#"{"title": "Bug report"}"#),
            thought_signature: None,
        };

        let actual = extract_tool_info(&fixture, &[]);

        assert_eq!(
            actual,
            Some(SummaryTool::Mcp { name: "mcp_github_create_issue".to_string() })
        );
    }

    #[test]
    fn test_summary_message_block_shell_helper() {
        let actual: SummaryMessage = SummaryToolCall::shell("cargo build").into();

        let expected = Block::ToolCall(SummaryToolCall {
            id: None,
            tool: SummaryTool::Shell { command: "cargo build".to_string() },
            is_success: true,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_links_shell_results_to_calls() {
        let fixture = context(vec![
            assistant_with_tools(
                "Running command",
                vec![ToolCatalog::tool_call_shell("echo test", "/test").call_id("call_1")],
            ),
            tool_result("shell", "call_1", false),
        ]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Running command"),
                SummaryToolCall::shell("echo test").id("call_1").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_mixed_file_and_shell_calls() {
        let fixture = context(vec![assistant_with_tools(
            "Multiple operations",
            vec![
                ToolCatalog::tool_call_read("/test/file.rs").call_id("call_1"),
                ToolCatalog::tool_call_shell("cargo test", "/test").call_id("call_2"),
                ToolCatalog::tool_call_write("/test/output.txt", "result").call_id("call_3"),
            ],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Multiple operations"),
                SummaryToolCall::read("/test/file.rs")
                    .id("call_1")
                    .is_success(false)
                    .into(),
                SummaryToolCall::shell("cargo test")
                    .id("call_2")
                    .is_success(false)
                    .into(),
                SummaryToolCall::update("/test/output.txt")
                    .id("call_3")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_ignores_non_file_tool_calls() {
        let fixture = context(vec![assistant_with_tools(
            "Searching",
            vec![ToolCallFull {
                name: ToolName::new("fs_search"),
                call_id: Some(ToolCallId::new("call_1")),
                arguments: ToolCallArguments::from_json(
                    r#"{"path": "/test", "pattern": "pattern"}"#,
                ),
                thought_signature: None,
            }],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Searching"),
                SummaryToolCall::search("pattern")
                    .id("call_1")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_summary_message_block_search_helper() {
        let actual: SummaryMessage = SummaryToolCall::search("/project/src").into();

        let expected = Block::ToolCall(SummaryToolCall {
            id: None,
            tool: SummaryTool::Search { pattern: "/project/src".to_string() },
            is_success: true,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_extracts_search_tool_calls() {
        let fixture = context(vec![assistant_with_tools(
            "Searching files",
            vec![ToolCatalog::tool_call_search("/test", "/test/src").call_id("call_1")],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Searching files"),
                SummaryToolCall::search("/test/src")
                    .id("call_1")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_extracts_codebase_search_tool_calls() {
        let fixture = context(vec![assistant_with_tools(
            "Searching codebase",
            vec![
                ToolCatalog::tool_call_semantic_search(vec![SearchQuery::new(
                    "retry mechanism",
                    "find retry logic",
                )])
                .call_id("call_1"),
            ],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Searching codebase"),
                SummaryToolCall::codebase_search(vec![SearchQuery::new(
                    "retry mechanism",
                    "find retry logic",
                )])
                .id("call_1")
                .is_success(false)
                .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_links_search_results_to_calls() {
        let fixture = context(vec![
            assistant_with_tools(
                "Searching",
                vec![ToolCatalog::tool_call_search("/test", "/test/src").call_id("call_1")],
            ),
            tool_result("search", "call_1", false),
        ]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Searching"),
                SummaryToolCall::search("/test/src").id("call_1").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_mixed_file_shell_and_search_calls() {
        let fixture = context(vec![assistant_with_tools(
            "Multiple operations",
            vec![
                ToolCatalog::tool_call_read("/test/file.rs").call_id("call_1"),
                ToolCatalog::tool_call_shell("cargo test", "/test").call_id("call_2"),
                ToolCatalog::tool_call_search("/test", "/test/src").call_id("call_3"),
                ToolCatalog::tool_call_write("/test/output.txt", "result").call_id("call_4"),
            ],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Multiple operations"),
                SummaryToolCall::read("/test/file.rs")
                    .id("call_1")
                    .is_success(false)
                    .into(),
                SummaryToolCall::shell("cargo test")
                    .id("call_2")
                    .is_success(false)
                    .into(),
                SummaryToolCall::search("/test/src")
                    .id("call_3")
                    .is_success(false)
                    .into(),
                SummaryToolCall::update("/test/output.txt")
                    .id("call_4")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_summary_message_block_undo_helper() {
        let actual: SummaryMessage = SummaryToolCall::undo("/test/file.rs").into();

        let expected = Block::ToolCall(SummaryToolCall {
            id: None,
            tool: SummaryTool::Undo { path: "/test/file.rs".to_string() },
            is_success: true,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_summary_message_block_fetch_helper() {
        let actual: SummaryMessage = SummaryToolCall::fetch("https://example.com").into();

        let expected = Block::ToolCall(SummaryToolCall {
            id: None,
            tool: SummaryTool::Fetch { url: "https://example.com".to_string() },
            is_success: true,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_summary_message_block_followup_helper() {
        let actual: SummaryMessage = SummaryToolCall::followup("What should I do next?").into();

        let expected = Block::ToolCall(SummaryToolCall {
            id: None,
            tool: SummaryTool::Followup { question: "What should I do next?".to_string() },
            is_success: true,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_summary_message_block_plan_helper() {
        let actual: SummaryMessage = SummaryToolCall::plan("feature-implementation").into();

        let expected = Block::ToolCall(SummaryToolCall {
            id: None,
            tool: SummaryTool::Plan { plan_name: "feature-implementation".to_string() },
            is_success: true,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_extracts_undo_tool_calls() {
        let fixture = context(vec![assistant_with_tools(
            "Undoing changes",
            vec![ToolCatalog::tool_call_undo("/test/file.rs").call_id("call_1")],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Undoing changes"),
                SummaryToolCall::undo("/test/file.rs")
                    .id("call_1")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_extracts_fetch_tool_calls() {
        let fixture = context(vec![assistant_with_tools(
            "Fetching data",
            vec![ToolCatalog::tool_call_fetch("https://api.example.com").call_id("call_1")],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Fetching data"),
                SummaryToolCall::fetch("https://api.example.com")
                    .id("call_1")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_extracts_followup_tool_calls() {
        let fixture = context(vec![assistant_with_tools(
            "Asking question",
            vec![ToolCatalog::tool_call_followup("Should I proceed?").call_id("call_1")],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Asking question"),
                SummaryToolCall::followup("Should I proceed?")
                    .id("call_1")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_extracts_plan_tool_calls() {
        let fixture = context(vec![assistant_with_tools(
            "Creating plan",
            vec![ToolCatalog::tool_call_plan("feature-plan", "v1", "test").call_id("call_1")],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Creating plan"),
                SummaryToolCall::plan("feature-plan")
                    .id("call_1")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_links_undo_results_to_calls() {
        let fixture = context(vec![
            assistant_with_tools(
                "Undoing",
                vec![ToolCatalog::tool_call_undo("/test/file.rs").call_id("call_1")],
            ),
            tool_result("undo", "call_1", false),
        ]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Undoing"),
                SummaryToolCall::undo("/test/file.rs").id("call_1").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_links_fetch_results_to_calls() {
        let fixture = context(vec![
            assistant_with_tools(
                "Fetching",
                vec![ToolCatalog::tool_call_fetch("https://example.com").call_id("call_1")],
            ),
            tool_result("fetch", "call_1", false),
        ]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Fetching"),
                SummaryToolCall::fetch("https://example.com")
                    .id("call_1")
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_links_followup_results_to_calls() {
        let fixture = context(vec![
            assistant_with_tools(
                "Asking",
                vec![ToolCatalog::tool_call_followup("Continue?").call_id("call_1")],
            ),
            tool_result("followup", "call_1", false),
        ]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Asking"),
                SummaryToolCall::followup("Continue?").id("call_1").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_links_plan_results_to_calls() {
        let fixture = context(vec![
            assistant_with_tools(
                "Planning",
                vec![ToolCatalog::tool_call_plan("my-plan", "v1", "test").call_id("call_1")],
            ),
            tool_result("plan", "call_1", false),
        ]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Planning"),
                SummaryToolCall::plan("my-plan").id("call_1").into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_all_tools_mixed() {
        let fixture = context(vec![assistant_with_tools(
            "All operations",
            vec![
                ToolCatalog::tool_call_read("/test/file.rs").call_id("call_1"),
                ToolCatalog::tool_call_write("/test/output.txt", "content").call_id("call_2"),
                ToolCatalog::tool_call_remove("/test/old.txt").call_id("call_3"),
                ToolCatalog::tool_call_shell("cargo build", "/test").call_id("call_4"),
                ToolCatalog::tool_call_search("/test", "/test/src").call_id("call_5"),
                ToolCatalog::tool_call_undo("/test/undo.txt").call_id("call_6"),
                ToolCatalog::tool_call_fetch("https://example.com").call_id("call_7"),
                ToolCatalog::tool_call_followup("Proceed?").call_id("call_8"),
                ToolCatalog::tool_call_plan("implementation", "v1", "test").call_id("call_9"),
            ],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("All operations"),
                SummaryToolCall::read("/test/file.rs")
                    .id("call_1")
                    .is_success(false)
                    .into(),
                SummaryToolCall::update("/test/output.txt")
                    .id("call_2")
                    .is_success(false)
                    .into(),
                SummaryToolCall::remove("/test/old.txt")
                    .id("call_3")
                    .is_success(false)
                    .into(),
                SummaryToolCall::shell("cargo build")
                    .id("call_4")
                    .is_success(false)
                    .into(),
                SummaryToolCall::search("/test/src")
                    .id("call_5")
                    .is_success(false)
                    .into(),
                SummaryToolCall::undo("/test/undo.txt")
                    .id("call_6")
                    .is_success(false)
                    .into(),
                SummaryToolCall::fetch("https://example.com")
                    .id("call_7")
                    .is_success(false)
                    .into(),
                SummaryToolCall::followup("Proceed?")
                    .id("call_8")
                    .is_success(false)
                    .into(),
                SummaryToolCall::plan("implementation")
                    .id("call_9")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_summary_message_block_mcp_helper() {
        let actual: SummaryMessage = SummaryToolCall::mcp("mcp_github_create_issue").into();

        let expected = Block::ToolCall(SummaryToolCall {
            id: None,
            tool: SummaryTool::Mcp { name: "mcp_github_create_issue".to_string() },
            is_success: true,
        });

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_extracts_mcp_tool_calls() {
        let fixture = context(vec![assistant_with_tools(
            "Creating GitHub issue",
            vec![ToolCallFull {
                name: ToolName::new("mcp_github_create_issue"),
                call_id: Some(ToolCallId::new("call_1")),
                arguments: ToolCallArguments::from_json(
                    r#"{"title": "Bug report", "body": "Description"}"#,
                ),
                thought_signature: None,
            }],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Creating GitHub issue"),
                SummaryToolCall::mcp("mcp_github_create_issue")
                    .id("call_1")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_links_mcp_results_to_calls() {
        let fixture = context(vec![
            assistant_with_tools(
                "Creating issue",
                vec![ToolCallFull {
                    name: ToolName::new("mcp_github_create_issue"),
                    call_id: Some(ToolCallId::new("call_1")),
                    arguments: ToolCallArguments::from_json(r#"{"title": "Bug"}"#),
                    thought_signature: None,
                }],
            ),
            tool_result("mcp_github_create_issue", "call_1", false),
        ]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Creating issue"),
                SummaryToolCall::mcp("mcp_github_create_issue")
                    .id("call_1")
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_multiple_mcp_tools() {
        let fixture = context(vec![assistant_with_tools(
            "Multiple MCP operations",
            vec![
                ToolCallFull {
                    name: ToolName::new("mcp_github_create_issue"),
                    call_id: Some(ToolCallId::new("call_1")),
                    arguments: ToolCallArguments::from_json(r#"{"title": "Bug"}"#),
                    thought_signature: None,
                },
                ToolCallFull {
                    name: ToolName::new("mcp_slack_post_message"),
                    call_id: Some(ToolCallId::new("call_2")),
                    arguments: ToolCallArguments::from_json(
                        r##"{"channel": "#dev", "text": "Hello"}"##,
                    ),
                    thought_signature: None,
                },
            ],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Multiple MCP operations"),
                SummaryToolCall::mcp("mcp_github_create_issue")
                    .id("call_1")
                    .is_success(false)
                    .into(),
                SummaryToolCall::mcp("mcp_slack_post_message")
                    .id("call_2")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_context_summary_mixed_system_and_mcp_tools() {
        let fixture = context(vec![assistant_with_tools(
            "Mixed operations",
            vec![
                ToolCatalog::tool_call_read("/test/file.rs").call_id("call_1"),
                ToolCallFull {
                    name: ToolName::new("mcp_github_create_issue"),
                    call_id: Some(ToolCallId::new("call_2")),
                    arguments: ToolCallArguments::from_json(r#"{"title": "Bug"}"#),
                    thought_signature: None,
                },
                ToolCatalog::tool_call_write("/test/output.txt", "result").call_id("call_3"),
            ],
        )]);

        let actual = ContextSummary::from(&fixture);

        let expected = ContextSummary::new(vec![SummaryBlock::new(
            Role::Assistant,
            vec![
                Block::content("Mixed operations"),
                SummaryToolCall::read("/test/file.rs")
                    .id("call_1")
                    .is_success(false)
                    .into(),
                SummaryToolCall::mcp("mcp_github_create_issue")
                    .id("call_2")
                    .is_success(false)
                    .into(),
                SummaryToolCall::update("/test/output.txt")
                    .id("call_3")
                    .is_success(false)
                    .into(),
            ],
        )]);

        assert_eq!(actual, expected);
    }
}
