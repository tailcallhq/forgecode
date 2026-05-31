use forge_domain::Transformer;

use crate::dto::anthropic::Request;
use crate::dto::anthropic::request::ToolEntry;

/// Transformer that capitalizes specific tool names for Anthropic
/// compatibility.
///
/// This transformer modifies tool names to use PascalCase for certain tools:
/// - `read` -> `Read`
/// - `write` -> `Write`
///
/// When the LLM sends back tool calls, both the capitalized and lowercase
/// versions are supported through alias handling in the deserialization logic.
pub struct CapitalizeToolNames;

impl Transformer for CapitalizeToolNames {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        for tool in &mut request.tools {
            if let ToolEntry::Function(tool) = tool {
                tool.name = match tool.name.as_str() {
                    "read" => "Read".to_string(),
                    "write" => "Write".to_string(),
                    "task" => "Task".to_string(),
                    _ => tool.name.clone(),
                };
            }
        }
        request
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Context, ContextMessage, ModelId, ToolDefinition, Transformer};

    use super::*;
    use crate::dto::anthropic::request::ToolEntry;

    /// Helper to extract the name from a ToolEntry::Function variant.
    fn tool_name(entry: &ToolEntry) -> &str {
        match entry {
            ToolEntry::Function(def) => &def.name,
            ToolEntry::WebSearch(ws) => &ws.name,
        }
    }

    #[test]
    fn test_capitalizes_read_tool() {
        let fixture = Context::default()
            .add_tool(ToolDefinition::new("read").description("Read a file"))
            .add_message(ContextMessage::user(
                "test",
                Some(ModelId::new("claude-3-5-sonnet-20241022")),
            ));

        let mut request = Request::try_from(fixture).unwrap();
        request = CapitalizeToolNames.transform(request);

        assert_eq!(tool_name(&request.tools[0]), "Read");
    }

    #[test]
    fn test_capitalizes_write_tool() {
        let fixture = Context::default()
            .add_tool(ToolDefinition::new("write").description("Write a file"))
            .add_message(ContextMessage::user(
                "test",
                Some(ModelId::new("claude-3-5-sonnet-20241022")),
            ));

        let mut request = Request::try_from(fixture).unwrap();
        request = CapitalizeToolNames.transform(request);

        assert_eq!(tool_name(&request.tools[0]), "Write");
    }

    #[test]
    fn test_leaves_other_tools_unchanged() {
        let fixture = Context::default()
            .add_tool(ToolDefinition::new("shell").description("Execute shell command"))
            .add_tool(ToolDefinition::new("fs_search").description("Search files"))
            .add_message(ContextMessage::user(
                "test",
                Some(ModelId::new("claude-3-5-sonnet-20241022")),
            ));

        let mut request = Request::try_from(fixture).unwrap();
        request = CapitalizeToolNames.transform(request);

        assert_eq!(tool_name(&request.tools[0]), "shell");
        assert_eq!(tool_name(&request.tools[1]), "fs_search");
    }

    #[test]
    fn test_handles_multiple_tools_including_read_and_write() {
        let fixture = Context::default()
            .add_tool(ToolDefinition::new("read").description("Read a file"))
            .add_tool(ToolDefinition::new("write").description("Write a file"))
            .add_tool(ToolDefinition::new("shell").description("Execute shell command"))
            .add_message(ContextMessage::user(
                "test",
                Some(ModelId::new("claude-3-5-sonnet-20241022")),
            ));

        let mut request = Request::try_from(fixture).unwrap();
        request = CapitalizeToolNames.transform(request);

        assert_eq!(tool_name(&request.tools[0]), "Read");
        assert_eq!(tool_name(&request.tools[1]), "Write");
        assert_eq!(tool_name(&request.tools[2]), "shell");
    }

    #[test]
    fn test_handles_empty_tools_list() {
        let fixture = Context::default().add_message(ContextMessage::user(
            "test",
            Some(ModelId::new("claude-3-5-sonnet-20241022")),
        ));

        let mut request = Request::try_from(fixture).unwrap();
        request = CapitalizeToolNames.transform(request);

        // No tools should be present
        assert_eq!(request.tools.len(), 0);
    }
}
