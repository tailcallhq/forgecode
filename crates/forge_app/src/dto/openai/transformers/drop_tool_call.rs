use forge_domain::Transformer;

use crate::dto::openai::{Request, Role};

/// Drops all tool call messages and converts them to user/assistant messages
pub struct DropToolCalls;

impl Transformer for DropToolCalls {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        if let Some(messages) = request.messages.as_mut() {
            for message in messages.iter_mut() {
                // Convert tool messages to user messages
                if message.role == Role::Tool {
                    message.role = Role::User;
                    message.tool_calls = None;
                    message.tool_call_id = None;
                    message.name = None;
                }
                // Remove tool calls from assistant messages
                if message.role == Role::Assistant {
                    message.tool_calls = None;
                }
            }
        }

        // Reset the tools field
        request.tools = None;

        request
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{
        Context, ContextMessage, Role, TextMessage, ToolCallFull, ToolCallId, ToolName, ToolResult,
    };

    use super::*;

    #[test]
    fn test_mistral_transformer_tools_not_supported() {
        let tool_call = ToolCallFull {
            call_id: Some(ToolCallId::new("123")),
            name: ToolName::new("test_tool"),
            arguments: serde_json::json!({"key": "value"}).into(),
            thought_signature: None,
        };

        let tool_result = ToolResult::new(ToolName::new("test_tool"))
            .call_id(ToolCallId::new("123"))
            .success("test result");

        let context = Context {
            conversation_id: None,
            messages: vec![
                ContextMessage::Text(
                    TextMessage::new(Role::Assistant, "Using tool").tool_calls(vec![tool_call]),
                )
                .into(),
                ContextMessage::Tool(tool_result).into(),
            ],
            tools: vec![forge_domain::ToolDefinition::new("test_tool").description("A test tool")],
            tool_choice: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            reasoning: None,
            stream: None,
            response_format: None,
            initiator: None,
        };

        let request = Request::from(context);
        let mut transformer = DropToolCalls;
        let transformed = transformer.transform(request);

        let messages = transformed.messages.unwrap();
        // Assistant message
        assert!(messages[0].tool_calls.is_none());
        // Converted tool message
        assert_eq!(messages[1].role, Role::User.into());
        // Tools field should be reset
        assert!(transformed.tools.is_none());
    }
}
