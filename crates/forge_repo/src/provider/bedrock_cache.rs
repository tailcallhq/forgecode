use aws_sdk_bedrockruntime::operation::converse_stream::ConverseStreamInput;
use aws_sdk_bedrockruntime::types::{
    CachePointBlock, CachePointType, ContentBlock, SystemContentBlock,
};
use forge_domain::Transformer;

/// Transformer that implements a simple two-breakpoint cache strategy for
/// Bedrock:
/// - Always caches after the first system message
/// - Always caches after the last message in the conversation
///
/// This follows AWS Bedrock's caching model where CachePoint blocks are
/// inserted at strategic positions to enable prompt caching.
pub struct SetCache;

// TODO: Implement it on Context or Conversation instead of ConverseStreamInput
impl Transformer for SetCache {
    type Value = ConverseStreamInput;

    /// Implements a simple two-breakpoint cache strategy:
    /// 1. Cache after the first system message (if exists)
    /// 2. Cache after the last message
    fn transform(&mut self, mut request: Self::Value) -> Self::Value {
        let sys_len = request.system.as_ref().map_or(0, |msgs| msgs.len());
        let msg_len = request.messages.as_ref().map_or(0, |msgs| msgs.len());

        if sys_len == 0 && msg_len == 0 {
            return request;
        }

        // Add cache point after first system message
        if let Some(system_messages) = request.system.as_mut()
            && !system_messages.is_empty()
        {
            system_messages.insert(
                1,
                SystemContentBlock::CachePoint(
                    CachePointBlock::builder()
                        .r#type(CachePointType::Default)
                        .build()
                        .expect("Failed to build CachePointBlock"),
                ),
            );
        }

        // Add cache point at the end of the last message's content
        if let Some(messages) = request.messages.as_mut()
            && let Some(last_message) = messages.last_mut()
        {
            last_message.content.push(ContentBlock::CachePoint(
                CachePointBlock::builder()
                    .r#type(CachePointType::Default)
                    .build()
                    .expect("Failed to build CachePointBlock"),
            ));
        }

        request
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{Context, ContextMessage, Role, TextMessage};
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::provider::FromDomain;

    fn has_cache_point_after_first_system(request: &ConverseStreamInput) -> bool {
        request
            .system
            .as_ref()
            .and_then(|sys| sys.get(1))
            .map(|block| matches!(block, SystemContentBlock::CachePoint(_)))
            .unwrap_or(false)
    }

    fn has_cache_point_in_last_message(request: &ConverseStreamInput) -> bool {
        request
            .messages
            .as_ref()
            .and_then(|msgs| msgs.last())
            .map(|msg| {
                msg.content
                    .iter()
                    .any(|block| matches!(block, ContentBlock::CachePoint(_)))
            })
            .unwrap_or(false)
    }

    #[test]
    fn test_single_user_message() {
        let context = Context {
            conversation_id: None,
            messages: vec![ContextMessage::Text(TextMessage::new(Role::User, "Hello")).into()],
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

        let request = ConverseStreamInput::from_domain(context).expect("Failed to convert context");
        let mut transformer = SetCache;
        let actual = transformer.transform(request);

        assert_eq!(has_cache_point_after_first_system(&actual), false);
        assert_eq!(has_cache_point_in_last_message(&actual), true);
    }

    #[test]
    fn test_with_system_message() {
        let context = Context {
            conversation_id: None,
            messages: vec![
                ContextMessage::Text(TextMessage::new(Role::System, "System prompt")).into(),
                ContextMessage::Text(TextMessage::new(Role::User, "Hello")).into(),
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

        let request = ConverseStreamInput::from_domain(context).expect("Failed to convert context");
        let mut transformer = SetCache;
        let actual = transformer.transform(request);

        assert_eq!(has_cache_point_after_first_system(&actual), true);
        assert_eq!(has_cache_point_in_last_message(&actual), true);
    }

    #[test]
    fn test_multiple_messages() {
        let context = Context {
            conversation_id: None,
            messages: vec![
                ContextMessage::Text(TextMessage::new(Role::System, "System prompt")).into(),
                ContextMessage::Text(TextMessage::new(Role::User, "Hello")).into(),
                ContextMessage::Text(TextMessage::new(Role::Assistant, "Hi there")).into(),
                ContextMessage::Text(TextMessage::new(Role::User, "How are you?")).into(),
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

        let request = ConverseStreamInput::from_domain(context).expect("Failed to convert context");
        let mut transformer = SetCache;
        let actual = transformer.transform(request);

        assert_eq!(has_cache_point_after_first_system(&actual), true);
        assert_eq!(has_cache_point_in_last_message(&actual), true);
    }
}
