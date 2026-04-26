use forge_domain::Transformer;

use crate::dto::openai::{Message, MessageContent, Request, Role};

/// Merges all system messages into a single system message at the beginning of
/// the messages array.
///
/// Some providers (e.g. NVIDIA) reject requests with multiple system messages
/// or system messages that are not positioned at the start of the conversation.
pub struct MergeSystemMessages;

impl Transformer for MergeSystemMessages {
    type Value = Request;

    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        if let Some(messages) = request.messages.take() {
            let (system, rest): (Vec<_>, Vec<_>) =
                messages.into_iter().partition(|m| m.role == Role::System);

            let merged = if system.is_empty() {
                rest
            } else {
                let combined_content = system
                    .iter()
                    .filter_map(|m| match &m.content {
                        Some(MessageContent::Text(text)) => Some(text.clone()),
                        Some(MessageContent::Parts(parts)) => Some(
                            parts
                                .iter()
                                .filter_map(|p| match p {
                                    crate::dto::openai::ContentPart::Text { text, .. } => {
                                        Some(text.clone())
                                    }
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join(""),
                        ),
                        None => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");

                if combined_content.is_empty() {
                    // All system messages had no content, don't create empty system message
                    rest
                } else {
                    let mut result = vec![Message {
                        role: Role::System,
                        content: Some(MessageContent::Text(combined_content)),
                        name: None,
                        tool_call_id: None,
                        tool_calls: None,
                        reasoning_details: None,
                        reasoning_text: None,
                        reasoning_opaque: None,
                        reasoning_content: None,
                        extra_content: None,
                    }];
                    result.extend(rest);
                    result
                }
            };

            request.messages = Some(merged);
        }
        request
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::dto::openai::{Message, MessageContent, Role};

    fn system_msg(content: &str) -> Message {
        Message {
            role: Role::System,
            content: Some(MessageContent::Text(content.to_string())),
            name: None,
            tool_call_id: None,
            tool_calls: None,
            reasoning_details: None,
            reasoning_text: None,
            reasoning_opaque: None,
            reasoning_content: None,
            extra_content: None,
        }
    }

    fn user_msg(content: &str) -> Message {
        Message {
            role: Role::User,
            content: Some(MessageContent::Text(content.to_string())),
            name: None,
            tool_call_id: None,
            tool_calls: None,
            reasoning_details: None,
            reasoning_text: None,
            reasoning_opaque: None,
            reasoning_content: None,
            extra_content: None,
        }
    }

    fn assistant_msg(content: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: Some(MessageContent::Text(content.to_string())),
            name: None,
            tool_call_id: None,
            tool_calls: None,
            reasoning_details: None,
            reasoning_text: None,
            reasoning_opaque: None,
            reasoning_content: None,
            extra_content: None,
        }
    }

    fn get_text_content(msg: &Message) -> Option<&str> {
        match msg.content.as_ref() {
            Some(MessageContent::Text(text)) => Some(text.as_str()),
            _ => None,
        }
    }

    #[test]
    fn test_multiple_system_messages_merged() {
        let fixture = Request::default().messages(vec![
            user_msg("hello"),
            system_msg("you are helpful"),
            assistant_msg("hi"),
            system_msg("be concise"),
            user_msg("how are you"),
        ]);

        let actual = MergeSystemMessages.transform(fixture);

        let messages = actual.messages.unwrap();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, Role::System);
        assert_eq!(
            get_text_content(&messages[0]),
            Some("you are helpful\n\nbe concise")
        );
        assert_eq!(messages[1].role, Role::User);
        assert_eq!(messages[2].role, Role::Assistant);
        assert_eq!(messages[3].role, Role::User);
    }

    #[test]
    fn test_single_system_message_unchanged() {
        let fixture = Request::default().messages(vec![
            system_msg("you are helpful"),
            user_msg("hello"),
            assistant_msg("hi"),
        ]);

        let actual = MergeSystemMessages.transform(fixture);

        let messages = actual.messages.unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, Role::System);
        assert_eq!(get_text_content(&messages[0]), Some("you are helpful"));
        assert_eq!(messages[1].role, Role::User);
        assert_eq!(messages[2].role, Role::Assistant);
    }

    #[test]
    fn test_no_system_messages_unchanged() {
        let fixture = Request::default().messages(vec![
            user_msg("hello"),
            assistant_msg("hi"),
            user_msg("how are you"),
        ]);

        let actual = MergeSystemMessages.transform(fixture);

        let messages = actual.messages.unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[1].role, Role::Assistant);
        assert_eq!(messages[2].role, Role::User);
    }

    #[test]
    fn test_no_messages_unchanged() {
        let fixture = Request::default();

        let actual = MergeSystemMessages.transform(fixture);

        assert!(actual.messages.is_none());
    }
}
