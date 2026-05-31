use super::Transformer;
use crate::{Context, ContextMessage};

/// Transformer that handles image processing in tool results
/// Converts image outputs from tool results into separate user messages with
/// image attachments
pub struct ImageHandling;

impl Default for ImageHandling {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageHandling {
    pub fn new() -> Self {
        Self
    }
}

impl Transformer for ImageHandling {
    type Value = Context;

    fn transform(&mut self, mut value: Self::Value) -> Self::Value {
        let mut images = Vec::new();

        // Step 1: Replace the image value with a text message
        value
            .messages
            .iter_mut()
            .filter_map(|message| {
                if let ContextMessage::Tool(tool_result) = &mut **message {
                    Some(tool_result)
                } else {
                    None
                }
            })
            .flat_map(|tool_result| tool_result.output.values.iter_mut())
            .for_each(|output_value| match output_value {
                crate::ToolValue::Image(image) => {
                    let image = std::mem::take(image);
                    let id = images.len();
                    *output_value = crate::ToolValue::Text(format!(
                        "[The image with ID {id} will be sent as an attachment in the next message]"
                    ));
                    images.push((id, image));
                }
                crate::ToolValue::Text(_) => {}
                crate::ToolValue::Empty => {}
                crate::ToolValue::AI { .. } => {}
                crate::ToolValue::Document(_) => {}
            });

        // Step 2: Insert all images at the end
        images.into_iter().for_each(|(id, image)| {
            value.messages.push(
                ContextMessage::user(format!("[Here is the image attachment for ID {id}]"), None)
                    .into(),
            );
            value.messages.push(ContextMessage::Image(image).into());
        });

        value
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_yaml_snapshot;
    use pretty_assertions::assert_eq;
    use serde::Serialize;

    use super::*;
    use crate::{Image, ToolCallId, ToolName, ToolOutput, ToolResult, ToolValue};

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

    fn create_context_with_multiple_images() -> Context {
        let image1 = Image::new_base64("image1_data".to_string(), "image/png");
        let image2 = Image::new_base64("image2_data".to_string(), "image/jpeg");

        Context::default()
            .add_message(ContextMessage::user("User message", None))
            .add_tool_results(vec![
                ToolResult {
                    name: ToolName::new("image_tool_1"),
                    call_id: Some(ToolCallId::new("call_1")),
                    output: ToolOutput::image(image1),
                },
                ToolResult {
                    name: ToolName::new("image_tool_2"),
                    call_id: Some(ToolCallId::new("call_2")),
                    output: ToolOutput::image(image2),
                },
            ])
    }

    #[test]
    fn test_image_handling_empty_context() {
        let fixture = Context::default();
        let mut transformer = ImageHandling::new();
        let actual = transformer.transform(fixture);
        let expected = Context::default();

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_image_handling_no_images() {
        let fixture = Context::default()
            .add_message(ContextMessage::system("System message"))
            .add_tool_results(vec![ToolResult {
                name: ToolName::new("text_tool"),
                call_id: Some(ToolCallId::new("call_text")),
                output: ToolOutput::text("Just text output".to_string()),
            }]);

        let mut transformer = ImageHandling::new();
        let actual = transformer.transform(fixture.clone());
        let expected = fixture;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_image_handling_single_image() {
        let fixture = create_context_with_multiple_images();
        let mut transformer = ImageHandling::new();
        let actual = transformer.transform(fixture.clone());

        let snapshot = TransformationSnapshot::new("ImageHandling", fixture, actual);
        assert_yaml_snapshot!(snapshot);
    }

    #[test]
    fn test_image_handling_multiple_images_in_single_tool_result() {
        let image1 = Image::new_base64("image1_data".to_string(), "image/png");
        let image2 = Image::new_base64("image2_data".to_string(), "image/jpeg");

        let fixture = Context::default().add_tool_results(vec![ToolResult {
            name: ToolName::new("multi_image_tool"),
            call_id: Some(ToolCallId::new("call_multi")),
            output: ToolOutput {
                values: vec![
                    ToolValue::Text("Before images".to_string()),
                    ToolValue::Image(image1),
                    ToolValue::Text("Between images".to_string()),
                    ToolValue::Image(image2),
                    ToolValue::Text("After images".to_string()),
                ],
                is_error: false,
            },
        }]);

        let mut transformer = ImageHandling::new();
        let actual = transformer.transform(fixture.clone());

        let snapshot = TransformationSnapshot::new("ImageHandling", fixture, actual);
        assert_yaml_snapshot!(snapshot);
    }

    #[test]
    fn test_image_handling_preserves_error_flag() {
        let image = Image::new_base64("error_image_data".to_string(), "image/png");

        let fixture = Context::default().add_tool_results(vec![ToolResult {
            name: ToolName::new("error_tool"),
            call_id: Some(ToolCallId::new("call_error")),
            output: ToolOutput {
                values: vec![
                    ToolValue::Text("Error occurred".to_string()),
                    ToolValue::Image(image),
                ],
                is_error: true,
            },
        }]);

        let mut transformer = ImageHandling::new();
        let actual = transformer.transform(fixture.clone());

        let snapshot = TransformationSnapshot::new("ImageHandling", fixture, actual);
        assert_yaml_snapshot!(snapshot);
    }

    #[test]
    fn test_image_handling_mixed_content_with_images() {
        let fixture = create_context_with_mixed_tool_outputs();
        let mut transformer = ImageHandling::new();
        let actual = transformer.transform(fixture.clone());

        let snapshot = TransformationSnapshot::new("ImageHandling", fixture, actual);
        assert_yaml_snapshot!(snapshot);
    }

    #[test]
    fn test_image_handling_preserves_non_tool_messages() {
        let image = Image::new_base64("test_image".to_string(), "image/png");

        let fixture = Context::default()
            .add_message(ContextMessage::system("System message"))
            .add_message(ContextMessage::user("User message", None))
            .add_message(ContextMessage::assistant(
                "Assistant message",
                None,
                None,
                None,
            ))
            .add_tool_results(vec![ToolResult {
                name: ToolName::new("image_tool"),
                call_id: Some(ToolCallId::new("call_preserve")),
                output: ToolOutput::image(image),
            }]);

        let mut transformer = ImageHandling::new();
        let actual = transformer.transform(fixture.clone());

        let snapshot = TransformationSnapshot::new("ImageHandling", fixture, actual);
        assert_yaml_snapshot!(snapshot);
    }
}
