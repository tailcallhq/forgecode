use forge_domain::Transformer;

use crate::dto::anthropic::Request;

/// Converts MCP tool names from Forge's internal format
/// (`mcp_{server}_tool_{tool}`) to the Claude Code compatible format
/// (`mcp__{server}__{tool}`).
///
/// Claude Code expects MCP tools to use double-underscore separators between
/// the `mcp` prefix, server name, and tool name. This transformer is applied
/// only when sending requests to the Claude Code provider (OAuth-authenticated
/// sessions).
pub struct McpToolNames;

impl Transformer for McpToolNames {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        for tool in &mut request.tools {
            tool.name = to_claude_code_format(&tool.name);
        }
        request
    }
}

/// Converts a tool name from the internal `mcp_{server}_tool_{tool}` format to
/// the Claude Code `mcp__{server}__{tool}` format.
///
/// Uses the last occurrence of `_tool_` as the server/tool separator to
/// correctly handle server names that themselves contain `_tool_` as a
/// substring. Names that do not match the internal format are returned
/// unchanged.
fn to_claude_code_format(name: &str) -> String {
    let Some(rest) = name.strip_prefix("mcp_") else {
        return name.to_string();
    };

    // Only convert names that contain the `_tool_` separator (internal format).
    // rsplit_once handles edge-case server names containing `_tool_`.
    if let Some((server, tool)) = rest.rsplit_once("_tool_") {
        return format!("mcp__{server}__{tool}");
    }

    name.to_string()
}

#[cfg(test)]
mod tests {
    use forge_domain::{Context, ContextMessage, ModelId, ToolDefinition, Transformer};
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_converts_simple_mcp_tool_name() {
        let actual = to_claude_code_format("mcp_github_tool_create_issue");
        let expected = "mcp__github__create_issue";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_converts_mcp_tool_name_with_underscored_server() {
        let actual = to_claude_code_format("mcp_hugging_face_tool_read_channel");
        let expected = "mcp__hugging_face__read_channel";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_handles_server_name_containing_tool_substring() {
        // Server named "my_tool_server", tool named "action"
        // rfind gives the LAST `_tool_`, so server = "my_tool_server", tool = "action"
        let actual = to_claude_code_format("mcp_my_tool_server_tool_action");
        let expected = "mcp__my_tool_server__action";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_leaves_already_converted_names_unchanged() {
        // mcp__ prefix means it was already converted; no `_tool_` in sanitized names
        let actual = to_claude_code_format("mcp__github__create_issue");
        let expected = "mcp__github__create_issue";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_leaves_non_mcp_tools_unchanged() {
        let actual = to_claude_code_format("read");
        let expected = "read";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_transformer_converts_mcp_tools_in_request() {
        let fixture = Context::default()
            .add_tool(
                ToolDefinition::new("mcp_github_tool_create_issue")
                    .description("Create a GitHub issue"),
            )
            .add_tool(ToolDefinition::new("read").description("Read a file"))
            .add_tool(
                ToolDefinition::new("mcp_slack_tool_send_message")
                    .description("Send a Slack message"),
            )
            .add_message(ContextMessage::user(
                "test",
                Some(ModelId::new("claude-3-5-sonnet-20241022")),
            ));

        let mut request = Request::try_from(fixture).unwrap();
        request = McpToolNames.transform(request);

        assert_eq!(request.tools[0].name, "mcp__github__create_issue");
        assert_eq!(request.tools[1].name, "read");
        assert_eq!(request.tools[2].name, "mcp__slack__send_message");
    }
}
