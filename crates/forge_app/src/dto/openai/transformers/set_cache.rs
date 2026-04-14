use forge_domain::Transformer;

use crate::dto::openai::Request;

/// Transformer that implements a simple two-breakpoint cache strategy:
/// - Always caches the first message in the conversation
/// - Always caches the last message in the conversation
/// - Removes cache control from the second-to-last message
pub struct SetCache;

impl Transformer for SetCache {
    type Value = Request;

    /// Implements a simple two-breakpoint cache strategy:
    /// 1. Cache the first message (index 0)
    /// 2. Cache the last message (index messages.len() - 1)
    /// 3. Remove cache control from second-to-last message (index
    ///    messages.len() - 2)
    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        if let Some(messages) = request.messages.as_mut() {
            let len = messages.len();

            if len == 0 {
                return request;
            }

            // Remove cache control from second-to-last message (when there are 3+ messages)
            if len >= 3
                && let Some(message) = messages.get_mut(len - 2)
                && let Some(ref content) = message.content
            {
                message.content = Some(content.clone().cached(false));
            }

            // Add cache control to first message
            if let Some(message) = messages.first_mut()
                && let Some(ref content) = message.content
            {
                message.content = Some(content.clone().cached(true));
            }

            // Add cache control to last message (if different from first)
            if let Some(message) = messages.last_mut()
                && let Some(ref content) = message.content
            {
                message.content = Some(content.clone().cached(true));
            }
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

    fn create_test_context(message: impl ToString) -> String {
        let context = Context {
            conversation_id: None,
            messages: message
                .to_string()
                .chars()
                .map(|c| match c {
                    's' => ContextMessage::Text(TextMessage::new(Role::System, c.to_string())),
                    'u' => ContextMessage::Text(
                        TextMessage::new(Role::User, c.to_string()).model(ModelId::new("gpt-4")),
                    ),
                    'a' => ContextMessage::Text(TextMessage::new(Role::Assistant, c.to_string())),
                    _ => {
                        panic!("Invalid character in test message");
                    }
                })
                .map(|msg| msg.into())
                .collect(),
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

        let request = Request::from(context);
        let mut transformer = SetCache;
        let request = transformer.transform(request);
        let mut output = String::new();
        let sequences = request
            .messages
            .into_iter()
            .flatten()
            .flat_map(|m| m.content)
            .enumerate()
            .filter(|(_, m)| m.is_cached())
            .map(|(i, _)| i)
            .collect::<HashSet<usize>>();

        for (i, c) in message.to_string().chars().enumerate() {
            if sequences.contains(&i) {
                output.push('[');
            }
            output.push_str(c.to_string().as_str())
        }

        output
    }

    #[test]
    fn test_single_message() {
        let actual = create_test_context("s");
        let expected = "[s";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_two_messages() {
        let actual = create_test_context("su");
        let expected = "[s[u";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_multiple_system_messages() {
        let actual = create_test_context("sssuuu");
        let expected = "[sssuu[u";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_three_messages_first_and_last_cached() {
        let actual = create_test_context("sua");
        let expected = "[su[a";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_four_messages_first_and_last_cached() {
        let actual = create_test_context("suau");
        let expected = "[sua[u";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_five_messages_first_and_last_cached() {
        let actual = create_test_context("suaua");
        let expected = "[suau[a";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_longer_conversation() {
        let actual = create_test_context("suuauuaaau");
        let expected = "[suuauuaaa[u";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_cache_removal_from_second_to_last() {
        // Test that second-to-last message doesn't have cache when there are 3+
        // messages
        let actual = create_test_context("suuauuaaauauau");
        let expected = "[suuauuaaauaua[u";
        assert_eq!(actual, expected);
    }
}
