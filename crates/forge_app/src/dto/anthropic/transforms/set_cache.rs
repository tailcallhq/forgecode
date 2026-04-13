use forge_domain::Transformer;

use crate::dto::anthropic::Request;

/// Transformer that implements a simple two-breakpoint cache strategy:
/// - Always caches the first message in the conversation
/// - Always caches the last message in the conversation
/// - Removes cache control from the second-to-last message
pub struct SetCache;

impl Transformer for SetCache {
    type Value = Request;

    /// Implements a simple two-breakpoint cache strategy:
    /// 1. Cache the first system message as it should be static.
    /// 2. Cache the last message (index messages.len() - 1)
    /// 3. Remove cache control from second-to-last message (index
    ///    messages.len() - 2)
    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        let len = request.get_messages().len();
        let sys_len = request.system.as_ref().map_or(0, |msgs| msgs.len());

        if len == 0 && sys_len == 0 {
            return request;
        }

        // Cache the very first system message, ideally you should keep static content
        // in it.
        if let Some(system_messages) = request.system.as_mut()
            && let Some(first_message) = system_messages.first_mut()
        {
            *first_message = std::mem::take(first_message).cached(true);
        } else {
            // If no system messages, we can still cache the first message in the
            // conversation.
            if let Some(first_message) = request.get_messages_mut().first_mut() {
                *first_message = std::mem::take(first_message).cached(true);
            }
        }

        // Add cache control to last message (if different from first)
        if let Some(message) = request.get_messages_mut().last_mut() {
            *message = std::mem::take(message).cached(true);
        }

        request
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use forge_domain::{Context, ContextMessage, ModelId, Role, TextMessage};
    use pretty_assertions::assert_eq;

    use super::*;

    fn create_test_context_with_system(
        system_messages: &str,
        conversation_messages: &str,
    ) -> String {
        let mut messages = Vec::new();

        // Add system messages to the regular messages array for Anthropic format
        for c in system_messages.chars() {
            match c {
                's' => messages.push(
                    ContextMessage::Text(TextMessage::new(Role::System, c.to_string())).into(),
                ),
                _ => panic!("Invalid character in system message: {}", c),
            }
        }

        // Add conversation messages
        for c in conversation_messages.chars() {
            match c {
                'u' => messages.push(
                    ContextMessage::Text(
                        TextMessage::new(Role::User, c.to_string())
                            .model(ModelId::new("claude-3-5-sonnet-20241022")),
                    )
                    .into(),
                ),
                'a' => messages.push(
                    ContextMessage::Text(TextMessage::new(Role::Assistant, c.to_string())).into(),
                ),
                _ => panic!("Invalid character in conversation message: {}", c),
            }
        }

        let context = Context {
            conversation_id: None,
            messages,
            tools: vec![],
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

        let request = Request::try_from(context).expect("Failed to convert context to request");
        let mut transformer = SetCache;
        let request = transformer.transform(request);

        let mut output = String::new();

        // Check if first system message is cached
        let system_cached = request
            .system
            .as_ref()
            .and_then(|sys| sys.first())
            .map(|msg| msg.is_cached())
            .unwrap_or(false);

        if system_cached {
            output.push('[');
        }
        output.push_str(system_messages);

        // Check which regular messages are cached
        let cached_indices = request
            .get_messages()
            .iter()
            .enumerate()
            .filter(|(_, m)| m.is_cached())
            .map(|(i, _)| i)
            .collect::<HashSet<usize>>();

        for (i, c) in conversation_messages.chars().enumerate() {
            if cached_indices.contains(&i) {
                output.push('[');
            }
            output.push(c);
        }

        output
    }

    fn create_test_context(message: impl ToString) -> String {
        create_test_context_with_system("", &message.to_string())
    }

    #[test]
    fn test_single_message() {
        let actual = create_test_context("u");
        let expected = "[u";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_two_messages() {
        let actual = create_test_context("ua");
        let expected = "[u[a";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_three_messages_only_last_cached() {
        let actual = create_test_context("uau");
        let expected = "[ua[u";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_four_messages_only_last_cached() {
        let actual = create_test_context("uaua");
        let expected = "[uau[a";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_five_messages_only_last_cached() {
        let actual = create_test_context("uauau");
        let expected = "[uaua[u";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_longer_conversation() {
        let actual = create_test_context("uauauauaua");
        let expected = "[uauauauau[a";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_with_system_message_single_conversation_message() {
        let actual = create_test_context_with_system("s", "u");
        let expected = "[s[u";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_with_system_message_multiple_conversation_messages() {
        let actual = create_test_context_with_system("ss", "uaua");
        let expected = "[ssuau[a";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_with_system_message_long_conversation() {
        let actual = create_test_context_with_system("s", "uauauauaua");
        let expected = "[suauauauau[a";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_only_system_message() {
        let actual = create_test_context_with_system("s", "");
        let expected = "[s";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_multiple_system_messages_only_first_cached() {
        // This test assumes multiple system messages are possible, but only first is
        // cached
        let context = Context {
            conversation_id: None,
            messages: vec![
                ContextMessage::Text(TextMessage::new(Role::System, "first")).into(),
                ContextMessage::Text(TextMessage::new(Role::System, "second")).into(),
                ContextMessage::Text(
                    TextMessage::new(Role::User, "user")
                        .model(ModelId::new("claude-3-5-sonnet-20241022")),
                )
                .into(),
            ],
            tools: vec![],
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

        let request = Request::try_from(context).expect("Failed to convert context to request");
        let mut transformer = SetCache;
        let request = transformer.transform(request);

        // Check that only first system message is cached
        let system_messages = request.system.as_ref().unwrap();
        assert_eq!(system_messages[0].is_cached(), true);
        assert_eq!(system_messages[1].is_cached(), false);

        // Check that last conversation message is cached
        assert_eq!(request.get_messages().last().unwrap().is_cached(), true);
    }
}
