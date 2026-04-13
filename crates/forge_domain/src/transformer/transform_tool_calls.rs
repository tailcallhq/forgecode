use super::Transformer;
use crate::{Context, ContextMessage, ModelId, Role, TextMessage};

pub struct TransformToolCalls {
    pub model: Option<ModelId>,
}

impl Default for TransformToolCalls {
    fn default() -> Self {
        Self::new()
    }
}

impl TransformToolCalls {
    pub fn new() -> Self {
        Self { model: None }
    }
}

impl Transformer for TransformToolCalls {
    type Value = Context;

    fn transform(&mut self, mut value: Self::Value) -> Self::Value {
        // This transformer converts a tool-supported context to a non-tool-supported
        // format We need to find assistant messages with tool calls and tool
        // result messages

        let mut new_messages = Vec::new();

        for message in value.messages.into_iter() {
            match &*message {
                ContextMessage::Text(text_msg)
                    if text_msg.role == Role::Assistant && text_msg.tool_calls.is_some() =>
                {
                    // Add the assistant message without tool calls
                    new_messages.push(
                        ContextMessage::Text(TextMessage {
                            role: text_msg.role,
                            content: text_msg.content.clone(),
                            raw_content: text_msg.raw_content.clone(),
                            tool_calls: None,
                            thought_signature: text_msg.thought_signature.clone(),
                            reasoning_details: text_msg.reasoning_details.clone(),
                            model: text_msg.model.clone(),
                            droppable: text_msg.droppable,
                            phase: text_msg.phase,
                        })
                        .into(),
                    );
                }
                ContextMessage::Tool(tool_result) => {
                    // Convert tool results to user messages
                    for output_value in tool_result.output.values.clone() {
                        match output_value {
                            crate::ToolValue::Text(text) => {
                                new_messages
                                    .push(ContextMessage::user(text, self.model.clone()).into());
                            }
                            crate::ToolValue::Image(image) => {
                                new_messages.push(ContextMessage::Image(image).into());
                            }
                            crate::ToolValue::Empty => {}
                            crate::ToolValue::AI { value, .. } => new_messages
                                .push(ContextMessage::user(value, self.model.clone()).into()),
                        }
                    }
                }
                _ => {
                    new_messages.push(message);
                }
            }
        }

        value.messages = new_messages;
        value.tools = Vec::new();
        value
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_yaml_snapshot;
    use pretty_assertions::assert_eq;
    use serde::Serialize;

    use super::*;
    use crate::{Image, ToolCallFull, ToolCallId, ToolName, ToolOutput, ToolResult, ToolValue};

    #[derive(Serialize)]
    struct TransformationSnapshot {
        transformation: String,
        before: Context,
        after: Context,
    }

    impl TransformationSnapshot {
        fn new(transformation: &str, before: Context, after: Context) -> Self {
            Self { transformation: transformation.to_string(), before, after }
        }
    }

    fn create_context_with_tool_calls() -> Context {
        let tool_call = ToolCallFull {
            name: ToolName::new("test_tool"),
            call_id: Some(ToolCallId::new("call_123")),
            arguments: serde_json::json!({"param": "value"}).into(),
            thought_signature: None,
        };

        Context::default()
            .add_message(ContextMessage::system("System message"))
            .add_message(ContextMessage::assistant(
                "I'll help you",
                None,
                None,
                Some(vec![tool_call]),
            ))
            .add_tool_results(vec![ToolResult {
                name: ToolName::new("test_tool"),
                call_id: Some(ToolCallId::new("call_123")),
                output: ToolOutput::text("Tool result text".to_string()),
            }])
    }

    fn create_context_with_mixed_tool_outputs() -> Context {
        let image = Image::new_base64("test_image_data".to_string(), "image/png");

        Context::default().add_tool_results(vec![ToolResult {
            name: ToolName::new("mixed_tool"),
            call_id: Some(ToolCallId::new("call_456")),
            output: ToolOutput {
                values: vec![
                    ToolValue::Text("First text output".to_string()),
                    ToolValue::Image(image),
                    ToolValue::Text("Second text output".to_string()),
                    ToolValue::Empty,
                ],
                is_error: false,
            },
        }])
    }

    #[test]
    fn test_transform_tool_calls_empty_context() {
        let fixture = Context::default();
        let mut transformer = TransformToolCalls::new();
        let actual = transformer.transform(fixture);
        let expected = Context::default();

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_transform_tool_calls_no_tool_calls() {
        let fixture = Context::default()
            .add_message(ContextMessage::system("System message"))
            .add_message(ContextMessage::user("User message", None))
            .add_message(ContextMessage::assistant(
                "Assistant response",
                None,
                None,
                None,
            ));

        let mut transformer = TransformToolCalls::new();
        let actual = transformer.transform(fixture.clone());
        let expected = fixture;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_transform_tool_calls_removes_tool_calls_from_assistant() {
        let fixture = create_context_with_tool_calls();
        let mut transformer = TransformToolCalls::new();
        let actual = transformer.transform(fixture.clone());

        let snapshot = TransformationSnapshot::new("TransformToolCalls", fixture, actual);
        assert_yaml_snapshot!(snapshot);
    }

    #[test]
    fn test_transform_tool_calls_with_model() {
        let fixture = create_context_with_tool_calls();
        let mut transformer = TransformToolCalls { model: Some(ModelId::new("gpt-4")) };
        let actual = transformer.transform(fixture.clone());

        let snapshot =
            TransformationSnapshot::new("TransformToolCalls::with_model(gpt-4)", fixture, actual);
        assert_yaml_snapshot!(snapshot);
    }

    #[test]
    fn test_transform_tool_calls_converts_tool_results_to_user_messages() {
        let fixture = create_context_with_mixed_tool_outputs();
        let mut transformer = TransformToolCalls::new();
        let actual = transformer.transform(fixture.clone());

        let snapshot = TransformationSnapshot::new("TransformToolCalls", fixture, actual);
        assert_yaml_snapshot!(snapshot);
    }

    #[test]
    fn test_transform_tool_calls_handles_empty_tool_outputs() {
        let fixture = Context::default().add_tool_results(vec![ToolResult {
            name: ToolName::new("empty_tool"),
            call_id: Some(ToolCallId::new("call_empty")),
            output: ToolOutput { values: vec![ToolValue::Empty], is_error: false },
        }]);

        let mut transformer = TransformToolCalls::new();
        let actual = transformer.transform(fixture.clone());

        let snapshot = TransformationSnapshot::new("TransformToolCalls", fixture, actual);
        assert_yaml_snapshot!(snapshot);
    }

    #[test]
    fn test_transform_tool_calls_clears_tools_field() {
        let fixture = Context::default()
            .add_tool(crate::ToolDefinition {
                name: crate::ToolName::new("test_tool"),
                description: "A test tool".to_string(),
                input_schema: schemars::schema_for!(()),
            })
            .add_message(ContextMessage::user("Test message", None));

        let mut transformer = TransformToolCalls::new();
        let actual = transformer.transform(fixture);

        assert_eq!(actual.tools.len(), 0);
    }
}
