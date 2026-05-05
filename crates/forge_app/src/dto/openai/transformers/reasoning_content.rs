use forge_domain::Transformer;

use crate::dto::openai::Request;

/// Transformer that converts structured reasoning details to the flat
/// `reasoning_content` format.
///
/// Some OpenAI-compatible providers expect replayed reasoning to be sent as a
/// flat `reasoning_content` string field instead of the OpenRouter-style
/// `reasoning_details` array.
///
/// This transformer:
/// 1. Extracts `reasoning.text` type entries into `reasoning_content`
/// 2. Removes `reasoning_details` because target providers do not accept it
pub struct ReasoningContent;

impl Transformer for ReasoningContent {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        if let Some(ref mut messages) = request.messages {
            for message in messages.iter_mut() {
                if let Some(reasoning_details) = &message.reasoning_details {
                    // Extract reasoning_content (type: "reasoning.text")
                    let reasoning_content = reasoning_details
                        .iter()
                        .find(|d| d.r#type == "reasoning.text")
                        .and_then(|d| d.text.clone());

                    // Set flat field
                    message.reasoning_content = reasoning_content;

                    // Remove reasoning_details array because target providers do not accept it
                    message.reasoning_details = None;
                }
            }
        }
        request
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::dto::openai::{Message, MessageContent, ReasoningDetail, Request, Role};

    #[test]
    fn test_converts_reasoning_details_to_reasoning_content() {
        // Setup
        let fixture = Request::default().messages(vec![Message {
            role: Role::Assistant,
            content: Some(MessageContent::Text("test".to_string())),
            reasoning_details: Some(vec![ReasoningDetail {
                r#type: "reasoning.text".to_string(),
                text: Some("thinking...".to_string()),
                signature: None,
                data: None,
                id: None,
                format: None,
                index: None,
            }]),
            reasoning_text: None,
            reasoning_opaque: None,
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
            extra_content: None,
        }]);

        // Execute
        let actual = ReasoningContent.transform(fixture);

        // Verify transformation
        let messages = actual.messages.unwrap();
        let msg = messages.first().unwrap();
        assert_eq!(msg.reasoning_content, Some("thinking...".to_string()));
        assert!(msg.reasoning_details.is_none());
    }

    #[test]
    fn test_handles_missing_reasoning_details() {
        // Setup
        let fixture = Request::default().messages(vec![Message {
            role: Role::User,
            content: Some(MessageContent::Text("test".to_string())),
            reasoning_details: None,
            reasoning_text: None,
            reasoning_opaque: None,
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
            extra_content: None,
        }]);

        // Execute
        let actual = ReasoningContent.transform(fixture.clone());

        // Verify - no change since there were no reasoning_details
        let messages = actual.messages.unwrap();
        let msg = messages.first().unwrap();
        assert!(msg.reasoning_content.is_none());
        assert!(msg.reasoning_details.is_none());
    }
}
