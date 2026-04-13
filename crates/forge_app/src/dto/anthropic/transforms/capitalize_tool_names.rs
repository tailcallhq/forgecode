use forge_domain::Transformer;

use crate::dto::anthropic::Request;

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
            tool.name = match tool.name.as_str() {
                "read" => "Read".to_string(),
                "write" => "Write".to_string(),
                "task" => "Task".to_string(),
                _ => tool.name.clone(),
            };
        }
        request
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Context, ContextMessage, ModelId, ToolDefinition, Transformer};

    use super::*;

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

        assert_eq!(request.tools[0].name, "Read");
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

        assert_eq!(request.tools[0].name, "Write");
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

        assert_eq!(request.tools[0].name, "shell");
        assert_eq!(request.tools[1].name, "fs_search");
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

        assert_eq!(request.tools[0].name, "Read");
        assert_eq!(request.tools[1].name, "Write");
        assert_eq!(request.tools[2].name, "shell");
    }

    #[test]
    fn test_handles_empty_tools_list() {
        let fixture = Context::default().add_message(ContextMessage::user(
            "test",
            Some(ModelId::new("claude-3-5-sonnet-20241022")),
        ));

        let mut request = Request::try_from(fixture).unwrap();
        request = CapitalizeToolNames.transform(request);

        assert!(request.tools.is_empty());
    }
}
