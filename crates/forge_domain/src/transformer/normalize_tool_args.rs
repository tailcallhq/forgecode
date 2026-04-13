use super::Transformer;
use crate::{Context, ContextMessage};

/// Normalizes tool call arguments before provider-specific request conversion.
///
/// This transformer repairs assistant tool calls that were persisted with
/// `Unparsed` string arguments, such as resumed conversations originating from
/// providers that emitted stringified or malformed JSON arguments. It converts
/// those arguments into `Parsed` JSON values so downstream DTO builders and
/// provider transforms operate on a consistent structure.
pub struct NormalizeToolCallArguments;

impl Default for NormalizeToolCallArguments {
    fn default() -> Self {
        Self::new()
    }
}

impl NormalizeToolCallArguments {
    pub fn new() -> Self {
        Self
    }
}

impl Transformer for NormalizeToolCallArguments {
    type Value = Context;

    fn transform(&mut self, mut value: Self::Value) -> Self::Value {
        // Iterate through all messages and normalize tool call arguments
        for entry in &mut value.messages {
            if let ContextMessage::Text(text_msg) = &mut entry.message
                && let Some(ref mut tool_calls) = text_msg.tool_calls
            {
                for tool_call in tool_calls.iter_mut() {
                    // Normalize the arguments - converts Unparsed JSON strings to Parsed
                    let args = std::mem::take(&mut tool_call.arguments);
                    tool_call.arguments = args.normalize();
                }
            }
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;
    use crate::{Role, TextMessage, ToolCallArguments, ToolCallFull, ToolCallId, ToolName};

    #[test]
    fn test_normalize_stringified_tool_call_arguments() {
        // Create a context with stringified tool call arguments (like from old dump)
        let context = Context::default()
            .add_message(ContextMessage::system("You are Forge."))
            .add_message(ContextMessage::Text(TextMessage {
                role: Role::Assistant,
                content: "I'll read the file.".to_string(),
                raw_content: None,
                tool_calls: Some(vec![ToolCallFull {
                    name: ToolName::new("read"),
                    call_id: Some(ToolCallId::new("call_001")),
                    // This is what an old dump would have - stringified JSON
                    arguments: ToolCallArguments::from_json(
                        r#"{"file_path": "/test/path", "start_line": 1}"#,
                    ),
                    thought_signature: None,
                }]),
                thought_signature: None,
                model: None,
                reasoning_details: None,
                droppable: false,
                phase: None,
            }));

        // Apply the transformer
        let mut transformer = NormalizeToolCallArguments::new();
        let normalized = transformer.transform(context);

        // Verify the tool call arguments are now Parsed
        let assistant_msg = normalized
            .messages
            .iter()
            .find_map(|entry| match &entry.message {
                ContextMessage::Text(text) if text.role == Role::Assistant => Some(text),
                _ => None,
            })
            .expect("Should find assistant message");

        let tool_calls = assistant_msg
            .tool_calls
            .as_ref()
            .expect("Should have tool calls");
        let tool_call = &tool_calls[0];

        // Arguments should now be Parsed, not Unparsed
        match &tool_call.arguments {
            ToolCallArguments::Parsed(value) => {
                assert_eq!(value["file_path"], "/test/path");
                assert_eq!(value["start_line"], 1);
            }
            ToolCallArguments::Unparsed(_) => {
                panic!("Arguments should be Parsed after normalization")
            }
        }

        // Serialize and verify it's a JSON object, not a string
        let serialized = serde_json::to_string(&normalized).expect("Should serialize");
        let reparsed: serde_json::Value =
            serde_json::from_str(&serialized).expect("Should re-parse");

        let messages = reparsed["messages"]
            .as_array()
            .expect("Should have messages");
        let assistant = messages
            .iter()
            .find(|m| m["text"]["role"] == "Assistant")
            .expect("Should find assistant");

        let args = &assistant["text"]["tool_calls"][0]["arguments"];
        assert!(
            args.is_object(),
            "Arguments must be JSON object for API, got: {}",
            args
        );
    }

    #[test]
    fn test_parsed_arguments_unchanged() {
        // Test that already Parsed arguments stay as Parsed
        let context = Context::default()
            .add_message(ContextMessage::system("You are Forge."))
            .add_message(ContextMessage::Text(TextMessage {
                role: Role::Assistant,
                content: "I'll read the file.".to_string(),
                raw_content: None,
                tool_calls: Some(vec![ToolCallFull {
                    name: ToolName::new("read"),
                    call_id: Some(ToolCallId::new("call_001")),
                    arguments: ToolCallArguments::Parsed(json!({
                        "file_path": "/test/path",
                        "start_line": 1
                    })),
                    thought_signature: None,
                }]),
                thought_signature: None,
                model: None,
                reasoning_details: None,
                droppable: false,
                phase: None,
            }));

        let mut transformer = NormalizeToolCallArguments::new();
        let normalized = transformer.transform(context);

        // Verify it's still Parsed and unchanged
        let assistant_msg = normalized
            .messages
            .iter()
            .find_map(|entry| match &entry.message {
                ContextMessage::Text(text) if text.role == Role::Assistant => Some(text),
                _ => None,
            })
            .expect("Should find assistant message");

        let tool_calls = assistant_msg
            .tool_calls
            .as_ref()
            .expect("Should have tool calls");

        match &tool_calls[0].arguments {
            ToolCallArguments::Parsed(value) => {
                assert_eq!(value["file_path"], "/test/path");
            }
            ToolCallArguments::Unparsed(_) => panic!("Should remain Parsed"),
        }
    }

    #[test]
    fn test_no_tool_calls_unchanged() {
        // Test that messages without tool calls are unchanged
        let context = Context::default()
            .add_message(ContextMessage::system("You are Forge."))
            .add_message(ContextMessage::user("Hello", None));

        let original = context.clone();
        let mut transformer = NormalizeToolCallArguments::new();
        let normalized = transformer.transform(context);

        // Should be unchanged
        assert_eq!(normalized.messages.len(), original.messages.len());
    }
}
