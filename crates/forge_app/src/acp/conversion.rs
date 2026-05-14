use std::path::PathBuf;

use agent_client_protocol as acp;
use forge_domain::{
    Agent, AgentId, Attachment, AttachmentContent, FileInfo, ToolCallFull, ToolName, ToolOutput,
    ToolValue,
};

use super::error::{Error, Result};

/// Maximum size in bytes for base64-encoded blob resources.
/// Protects against OOM from oversized client payloads.
const MAX_BLOB_SIZE: usize = 50 * 1024 * 1024; // 50 MB

/// Maps a Forge tool name to an ACP ToolKind.
///
/// Native Forge tools are classified by exact match. MCP tools (prefixed
/// with `mcp_`) use best-effort keyword heuristics and default to `Other`
/// when the name is ambiguous. The heuristic is order-dependent: the first
/// matching keyword category wins.
pub(crate) fn map_tool_kind(tool_name: &ToolName) -> acp::ToolKind {
    match tool_name.as_str() {
        "read" => acp::ToolKind::Read,
        "write" | "patch" => acp::ToolKind::Edit,
        "remove" | "undo" => acp::ToolKind::Delete,
        "fs_search" | "sem_search" => acp::ToolKind::Search,
        "shell" => acp::ToolKind::Execute,
        "fetch" => acp::ToolKind::Fetch,
        "sage" => acp::ToolKind::Think,
        _ => classify_mcp_tool(tool_name.as_str()),
    }
}

/// Best-effort classification for MCP tools by keyword heuristic.
///
/// Falls back to `Other` for non-MCP tools or when no keyword matches.
/// The match order matters: a tool named `mcp_get_search_results` would
/// classify as `Read` (matches "get" before "search").
fn classify_mcp_tool(name: &str) -> acp::ToolKind {
    if !name.starts_with("mcp_") {
        return acp::ToolKind::Other;
    }

    // Strip the "mcp_<server>_" prefix to get the action portion.
    // E.g. "mcp_github_list_issues" → check against "list_issues".
    let action = name
        .strip_prefix("mcp_")
        .and_then(|rest| rest.split_once('_').map(|(_, action)| action))
        .unwrap_or(name);

    const READ_KEYWORDS: &[&str] = &["read", "get", "fetch", "list", "show", "view", "load"];
    const SEARCH_KEYWORDS: &[&str] = &["search", "query", "find", "filter", "lookup"];
    const EDIT_KEYWORDS: &[&str] = &[
        "write", "update", "create", "set", "add", "insert", "push", "merge",
        "fork", "comment", "assign", "request",
    ];
    const DELETE_KEYWORDS: &[&str] = &["delete", "remove", "drop", "clear", "close", "cancel"];
    const EXECUTE_KEYWORDS: &[&str] = &["execute", "run", "start", "invoke", "call"];

    let checks: &[(&[&str], acp::ToolKind)] = &[
        (READ_KEYWORDS, acp::ToolKind::Read),
        (SEARCH_KEYWORDS, acp::ToolKind::Search),
        (EDIT_KEYWORDS, acp::ToolKind::Edit),
        (DELETE_KEYWORDS, acp::ToolKind::Delete),
        (EXECUTE_KEYWORDS, acp::ToolKind::Execute),
    ];

    for (keywords, kind) in checks {
        if keywords.iter().any(|kw| action.contains(kw)) {
            return kind.clone();
        }
    }

    acp::ToolKind::Other
}

pub(crate) fn extract_file_locations(
    tool_name: &ToolName,
    arguments: &serde_json::Value,
) -> Vec<acp::ToolCallLocation> {
    match tool_name.as_str() {
        "read" | "write" | "patch" | "remove" | "undo" => arguments
            .get("file_path")
            .and_then(|value| value.as_str())
            .map(|file_path| vec![acp::ToolCallLocation::new(PathBuf::from(file_path))])
            .unwrap_or_default(),
        _ => vec![],
    }
}

pub(crate) fn map_tool_call_to_acp(tool_call: &ToolCallFull) -> acp::ToolCall {
    let tool_call_id = tool_call
        .call_id
        .as_ref()
        .map(|id| id.as_str().to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let locations = extract_file_locations(
        &tool_call.name,
        &serde_json::to_value(&tool_call.arguments).unwrap_or(serde_json::json!({})),
    );

    acp::ToolCall::new(tool_call_id, tool_call.name.as_str().to_string())
        .kind(map_tool_kind(&tool_call.name))
        .status(acp::ToolCallStatus::Pending)
        .locations(locations)
        .raw_input(
            serde_json::to_value(&tool_call.arguments)
                .ok()
                .filter(|value| !value.is_null()),
        )
}

/// Converts a ToolOutput into ACP content blocks.
pub(crate) fn convert_tool_output(output: &ToolOutput) -> Vec<acp::ToolCallContent> {
    output
        .values
        .iter()
        .filter_map(convert_tool_value)
        .collect()
}

fn convert_tool_value(value: &ToolValue) -> Option<acp::ToolCallContent> {
    match value {
        ToolValue::Text(text) => convert_text(text),
        ToolValue::AI { value, .. } => convert_text(value),
        ToolValue::Image(image) => Some(acp::ToolCallContent::Content(acp::Content::new(
            acp::ContentBlock::Image(acp::ImageContent::new(image.data(), image.mime_type())),
        ))),
        ToolValue::Empty => None,
    }
}

fn convert_text(text: &str) -> Option<acp::ToolCallContent> {
    if text.is_empty() {
        None
    } else {
        Some(acp::ToolCallContent::Content(acp::Content::new(
            acp::ContentBlock::Text(acp::TextContent::new(text.to_string())),
        )))
    }
}

pub(crate) fn acp_resource_to_attachment(resource: &acp::EmbeddedResource) -> Result<Attachment> {
    let (content_text, uri) = match &resource.resource {
        acp::EmbeddedResourceResource::TextResourceContents(text_resource) => {
            (text_resource.text.clone(), text_resource.uri.clone())
        }
        acp::EmbeddedResourceResource::BlobResourceContents(blob_resource) => {
            if blob_resource.blob.len() > MAX_BLOB_SIZE {
                return Err(Error::Application(anyhow::anyhow!(
                    "Blob resource exceeds maximum size of {} bytes",
                    MAX_BLOB_SIZE
                )));
            }
            let decoded = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                &blob_resource.blob,
            )
            .map_err(|error| {
                Error::Application(anyhow::anyhow!("Failed to decode base64 blob: {}", error))
            })?;
            let text = String::from_utf8(decoded).map_err(|error| {
                Error::Application(anyhow::anyhow!("Failed to decode UTF-8: {}", error))
            })?;
            (text, blob_resource.uri.clone())
        }
        _ => {
            return Err(Error::Application(anyhow::anyhow!(
                "Unsupported resource type"
            )))
        }
    };

    let path = uri_to_path(&uri);
    let total_lines = content_text.lines().count() as u64;
    let info = FileInfo::new(1, total_lines, total_lines, String::new());
    let content = AttachmentContent::FileContent {
        content: content_text,
        info,
    };

    Ok(Attachment { path, content })
}

pub(crate) fn uri_to_path(uri: &str) -> String {
    if let Some(path) = uri.strip_prefix("file://") {
        if path.len() > 2 && path.chars().nth(2) == Some(':') {
            path.trim_start_matches('/').to_string()
        } else {
            path.to_string()
        }
    } else {
        uri.to_string()
    }
}

pub(crate) fn build_session_mode_state(
    agents: &[Agent],
    current_agent_id: &AgentId,
) -> acp::SessionModeState {
    let available_modes = agents
        .iter()
        .map(|agent| {
            acp::SessionMode::new(
                acp::SessionModeId::new(agent.id.to_string()),
                agent.id.to_string(),
            )
            .description(agent.description.clone())
        })
        .collect();

    acp::SessionModeState::new(
        acp::SessionModeId::new(current_agent_id.to_string()),
        available_modes,
    )
}

#[cfg(test)]
mod tests {
    use forge_domain::{ConversationId, Image};
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_uri_to_path_preserves_non_file_uri() {
        let fixture = "relative/path.txt";
        let actual = uri_to_path(fixture);
        let expected = "relative/path.txt".to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_uri_to_path_strips_file_prefix() {
        let fixture = "file:///home/user/file.txt";
        let actual = uri_to_path(fixture);
        let expected = "/home/user/file.txt".to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_markdown_sent_to_acp_not_xml() {
        let fixture = ToolOutput::text("## File: test.txt\n\nContent here");

        let actual = convert_tool_output(&fixture);

        assert_eq!(actual.len(), 1);
        if let Some(acp::ToolCallContent::Content(content)) = actual.first() {
            if let acp::ContentBlock::Text(text) = &content.content {
                assert_eq!(text.text, "## File: test.txt\n\nContent here");
            } else {
                panic!("Expected text content block");
            }
        } else {
            panic!("Expected content");
        }
    }

    #[test]
    fn test_ai_output_sent_to_acp_as_text() {
        let fixture = ToolOutput::ai(ConversationId::generate(), "Agent result");

        let actual = convert_tool_output(&fixture);

        assert_eq!(actual.len(), 1);
        if let Some(acp::ToolCallContent::Content(content)) = actual.first() {
            if let acp::ContentBlock::Text(text) = &content.content {
                assert_eq!(text.text, "Agent result");
            } else {
                panic!("Expected text content block");
            }
        } else {
            panic!("Expected content");
        }
    }

    #[test]
    fn test_image_sent_to_acp() {
        let image = Image::new_bytes(vec![1, 2, 3, 4], "image/png".to_string());
        let fixture = ToolOutput::image(image);

        let actual = convert_tool_output(&fixture);

        assert_eq!(actual.len(), 1);
        if let Some(acp::ToolCallContent::Content(content)) = actual.first() {
            assert!(matches!(content.content, acp::ContentBlock::Image(_)));
        } else {
            panic!("Expected content");
        }
    }

    #[test]
    fn test_empty_output_produces_no_content() {
        let fixture = ToolOutput::text("");
        let actual = convert_tool_output(&fixture);
        let expected: Vec<acp::ToolCallContent> = vec![];
        assert_eq!(actual.len(), expected.len());
    }

    #[test]
    fn test_map_tool_kind_native_tools() {
        let fixture = ToolName::new("read");
        let actual = map_tool_kind(&fixture);
        assert!(matches!(actual, acp::ToolKind::Read));
    }

    #[test]
    fn test_map_tool_kind_mcp_read() {
        let fixture = ToolName::new("mcp_github_list_issues");
        let actual = map_tool_kind(&fixture);
        assert!(matches!(actual, acp::ToolKind::Read));
    }

    #[test]
    fn test_map_tool_kind_mcp_search() {
        let fixture = ToolName::new("mcp_db_search_records");
        let actual = map_tool_kind(&fixture);
        assert!(matches!(actual, acp::ToolKind::Search));
    }

    #[test]
    fn test_map_tool_kind_unknown_defaults_to_other() {
        let fixture = ToolName::new("mcp_custom_foobar");
        let actual = map_tool_kind(&fixture);
        assert!(matches!(actual, acp::ToolKind::Other));
    }

    #[test]
    fn test_map_tool_kind_non_mcp_unknown() {
        let fixture = ToolName::new("custom_tool");
        let actual = map_tool_kind(&fixture);
        assert!(matches!(actual, acp::ToolKind::Other));
    }

    #[test]
    fn test_extract_file_locations_read_tool() {
        let fixture_name = ToolName::new("read");
        let fixture_args = serde_json::json!({"file_path": "/tmp/test.rs"});
        let actual = extract_file_locations(&fixture_name, &fixture_args);
        assert_eq!(actual.len(), 1);
    }

    #[test]
    fn test_extract_file_locations_unknown_tool() {
        let fixture_name = ToolName::new("shell");
        let fixture_args = serde_json::json!({"command": "ls"});
        let actual = extract_file_locations(&fixture_name, &fixture_args);
        let expected: Vec<acp::ToolCallLocation> = vec![];
        assert_eq!(actual.len(), expected.len());
    }
}
