use forge_domain::Transformer;

use crate::dto::openai::Request;

/// Transformer that ensures every message has a `reasoning_content` field set.
///
/// DeepSeek requires the `reasoning_content` field to be present on assistant
/// messages even when there is no reasoning text. When the field is `None` this
/// transformer falls back to an empty string.
pub struct DefaultReasoningContent;

impl Transformer for DefaultReasoningContent {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        if let Some(ref mut messages) = request.messages {
            for message in messages.iter_mut() {
                if message.reasoning_content.is_none() {
                    message.reasoning_content = Some(String::new());
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
    use crate::dto::openai::{Message, MessageContent, Request, Role};

    #[test]
    fn test_sets_empty_string_when_reasoning_content_is_none() {
        let fixture = Request::default().messages(vec![Message {
            role: Role::Assistant,
            content: Some(MessageContent::Text("test".to_string())),
            name: None,
            tool_call_id: None,
            tool_calls: None,
            reasoning_details: None,
            reasoning_text: None,
            reasoning_opaque: None,
            reasoning_content: None,
            extra_content: None,
        }]);

        let actual = DefaultReasoningContent.transform(fixture);

        let messages = actual.messages.unwrap();
        let msg = messages.first().unwrap();
        assert_eq!(msg.reasoning_content, Some(String::new()));
    }

    #[test]
    fn test_preserves_existing_reasoning_content() {
        let fixture = Request::default().messages(vec![Message {
            role: Role::Assistant,
            content: Some(MessageContent::Text("test".to_string())),
            name: None,
            tool_call_id: None,
            tool_calls: None,
            reasoning_details: None,
            reasoning_text: None,
            reasoning_opaque: None,
            reasoning_content: Some("thinking...".to_string()),
            extra_content: None,
        }]);

        let actual = DefaultReasoningContent.transform(fixture);

        let messages = actual.messages.unwrap();
        let msg = messages.first().unwrap();
        assert_eq!(msg.reasoning_content, Some("thinking...".to_string()));
    }

    #[test]
    fn test_preserves_preserves_empty_string_when_already_set() {
        let fixture = Request::default().messages(vec![Message {
            role: Role::Assistant,
            content: Some(MessageContent::Text("test".to_string())),
            name: None,
            tool_call_id: None,
            tool_calls: None,
            reasoning_details: None,
            reasoning_text: None,
            reasoning_opaque: None,
            reasoning_content: Some(String::new()),
            extra_content: None,
        }]);

        let actual = DefaultReasoningContent.transform(fixture);

        let messages = actual.messages.unwrap();
        let msg = messages.first().unwrap();
        assert_eq!(msg.reasoning_content, Some(String::new()));
    }
}
