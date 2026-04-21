use forge_domain::Transformer;

use crate::dto::anthropic::Request;

/// Transformer that keeps Anthropic prompt-cache markers stable:
/// - Always caches every system message so the static system prefix remains
///   reusable
/// - Falls back to caching the first conversation message when there is no
///   system prompt so single-turn requests still establish a reusable prefix
/// - Uses exactly one rolling message-level marker on the newest message
pub struct SetCache;

impl Transformer for SetCache {
    type Value = Request;

    /// Applies the default Anthropic cache strategy:
    /// 1. Cache every system message when present, otherwise cache the first
    ///    conversation message.
    /// 2. Cache only the last message as the rolling message-level marker.
    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        let len = request.get_messages().len();
        let sys_len = request.system.as_ref().map_or(0, |msgs| msgs.len());

        if len == 0 && sys_len == 0 {
            return request;
        }

        let has_system_prompt = request
            .system
            .as_ref()
            .is_some_and(|messages| !messages.is_empty());

        if let Some(system_messages) = request.system.as_mut() {
            for message in system_messages.iter_mut() {
                *message = std::mem::take(message).cached(true);
            }
        }

        for message in request.get_messages_mut().iter_mut() {
            *message = std::mem::take(message).cached(false);
        }

        if !has_system_prompt
            && len > 0
            && let Some(first_message) = request.get_messages_mut().first_mut()
        {
            *first_message = std::mem::take(first_message).cached(true);
        }

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
    fn test_three_messages_cache_first_and_last_only() {
        let actual = create_test_context("uau");
        let expected = "[ua[u";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_four_messages_cache_first_and_last_only() {
        let actual = create_test_context("uaua");
        let expected = "[uau[a";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_five_messages_cache_first_and_last_only() {
        let actual = create_test_context("uauau");
        let expected = "[uaua[u";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_longer_conversation_caches_first_and_last_only() {
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
    fn test_multiple_system_messages_all_cached() {
        let fixture = Context {
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

        let request = Request::try_from(fixture).expect("Failed to convert context to request");
        let mut transformer = SetCache;
        let request = transformer.transform(request);

        let expected = vec![true, true];
        let actual = request
            .system
            .as_ref()
            .unwrap()
            .iter()
            .map(|message| message.is_cached())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        assert!(request.get_messages()[0].is_cached());
    }
}
