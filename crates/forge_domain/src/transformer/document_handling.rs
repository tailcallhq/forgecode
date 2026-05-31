use super::Transformer;
use crate::{Context, ContextMessage};

/// Transformer that handles document processing in tool results.
///
/// Converts document outputs from tool results into separate user messages
/// with document attachments, following the same pattern as `ImageHandling`.
pub struct DocumentHandling;

impl Default for DocumentHandling {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentHandling {
    pub fn new() -> Self {
        Self
    }
}

impl Transformer for DocumentHandling {
    type Value = Context;

    fn transform(&mut self, mut value: Self::Value) -> Self::Value {
        let mut documents = Vec::new();

        // Step 1: Replace document values with text placeholders
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
            .for_each(|output_value| {
                if let crate::ToolValue::Document(doc) = output_value {
                    let doc = std::mem::take(doc);
                    let id = documents.len();
                    *output_value = crate::ToolValue::Text(format!(
                        "[The document with ID {id} will be sent as an attachment in the next message]"
                    ));
                    documents.push((id, doc));
                }
            });

        // Step 2: Insert all documents at the end
        documents.into_iter().for_each(|(id, doc)| {
            value.messages.push(
                ContextMessage::user(
                    format!("[Here is the document attachment for ID {id}]"),
                    None,
                )
                .into(),
            );
            value.messages.push(ContextMessage::Document(doc).into());
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
    use crate::{Document, ToolCallId, ToolName, ToolOutput, ToolResult, ToolValue};

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

    #[test]
    fn test_document_handling_empty_context() {
        let fixture = Context::default();
        let mut transformer = DocumentHandling::new();
        let actual = transformer.transform(fixture);
        let expected = Context::default();

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_document_handling_no_documents() {
        let fixture = Context::default()
            .add_message(ContextMessage::system("System message"))
            .add_tool_results(vec![ToolResult {
                name: ToolName::new("text_tool"),
                call_id: Some(ToolCallId::new("call_text")),
                output: ToolOutput::text("Just text output".to_string()),
            }]);

        let mut transformer = DocumentHandling::new();
        let actual = transformer.transform(fixture.clone());
        let expected = fixture;

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_document_handling_single_document() {
        let doc = Document::new_base64("pdf_data".to_string(), "application/pdf");
        let fixture = Context::default().add_tool_results(vec![ToolResult {
            name: ToolName::new("read_tool"),
            call_id: Some(ToolCallId::new("call_1")),
            output: ToolOutput::document(doc),
        }]);

        let mut transformer = DocumentHandling::new();
        let actual = transformer.transform(fixture.clone());

        let snapshot = TransformationSnapshot::new("DocumentHandling", fixture, actual);
        assert_yaml_snapshot!(snapshot);
    }

    #[test]
    fn test_document_handling_mixed_content() {
        let doc = Document::new_base64("pdf_data".to_string(), "application/pdf");
        let fixture = Context::default().add_tool_results(vec![ToolResult {
            name: ToolName::new("mixed_tool"),
            call_id: Some(ToolCallId::new("call_1")),
            output: ToolOutput {
                values: vec![
                    ToolValue::Text("Before document".to_string()),
                    ToolValue::Document(doc),
                    ToolValue::Text("After document".to_string()),
                ],
                is_error: false,
            },
        }]);

        let mut transformer = DocumentHandling::new();
        let actual = transformer.transform(fixture.clone());

        let snapshot = TransformationSnapshot::new("DocumentHandling", fixture, actual);
        assert_yaml_snapshot!(snapshot);
    }

    #[test]
    fn test_document_handling_preserves_non_tool_messages() {
        let doc = Document::new_base64("pdf_data".to_string(), "application/pdf");
        let fixture = Context::default()
            .add_message(ContextMessage::system("System message"))
            .add_message(ContextMessage::user("User message", None))
            .add_tool_results(vec![ToolResult {
                name: ToolName::new("doc_tool"),
                call_id: Some(ToolCallId::new("call_1")),
                output: ToolOutput::document(doc),
            }]);

        let mut transformer = DocumentHandling::new();
        let actual = transformer.transform(fixture.clone());

        let snapshot = TransformationSnapshot::new("DocumentHandling", fixture, actual);
        assert_yaml_snapshot!(snapshot);
    }
}
