use forge_domain::Transformer;

use crate::dto::openai::Request;

/// Transformer that converts reasoning_details to GitHub Copilot's flat format.
///
/// GitHub Copilot uses `reasoning_text` and `reasoning_opaque` fields directly
/// in messages instead of the OpenRouter-style `reasoning_details` array.
///
/// This transformer:
/// 1. Extracts `reasoning.text` type entries → `reasoning_text`
/// 2. Extracts `reasoning.encrypted` type entries → `reasoning_opaque`
/// 3. Removes `reasoning_details` array
pub struct GitHubCopilotReasoning;

impl Transformer for GitHubCopilotReasoning {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        if let Some(ref mut messages) = request.messages {
            for message in messages.iter_mut() {
                if let Some(reasoning_details) = &message.reasoning_details {
                    // Extract reasoning_text (type: "reasoning.text")
                    let reasoning_text = reasoning_details
                        .iter()
                        .find(|d| d.r#type == "reasoning.text")
                        .and_then(|d| d.text.clone());

                    // Extract reasoning_opaque (type: "reasoning.encrypted")
                    let reasoning_opaque = reasoning_details
                        .iter()
                        .find(|d| d.r#type == "reasoning.encrypted")
                        .and_then(|d| d.data.clone());

                    // Set flat fields
                    message.reasoning_text = reasoning_text;
                    message.reasoning_opaque = reasoning_opaque;

                    // Remove reasoning_details array (GitHub Copilot doesn't
                    // accept it)
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
    fn test_converts_reasoning_details_to_flat_format() {
        // Setup
        let fixture = Request::default().messages(vec![Message {
            role: Role::Assistant,
            content: Some(MessageContent::Text("test".to_string())),
            reasoning_details: Some(vec![
                ReasoningDetail {
                    r#type: "reasoning.text".to_string(),
                    text: Some("thinking...".to_string()),
                    signature: None,
                    data: None,
                    id: None,
                    format: None,
                    index: None,
                },
                ReasoningDetail {
                    r#type: "reasoning.encrypted".to_string(),
                    text: None,
                    signature: None,
                    data: Some("encrypted_data".to_string()),
                    id: None,
                    format: None,
                    index: None,
                },
            ]),
            reasoning_text: None,
            reasoning_opaque: None,
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
            extra_content: None,
        }]);

        // Execute
        let actual = GitHubCopilotReasoning.transform(fixture);

        // Verify transformation
        let messages = actual.messages.unwrap();
        let msg = messages.first().unwrap();
        assert_eq!(msg.reasoning_text, Some("thinking...".to_string()));
        assert_eq!(msg.reasoning_opaque, Some("encrypted_data".to_string()));
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
        let actual = GitHubCopilotReasoning.transform(fixture.clone());

        // Verify - no change since there were no reasoning_details
        let messages = actual.messages.unwrap();
        let msg = messages.first().unwrap();
        assert!(msg.reasoning_text.is_none());
        assert!(msg.reasoning_opaque.is_none());
    }
}
