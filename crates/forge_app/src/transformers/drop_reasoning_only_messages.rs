use forge_domain::{Context, ContextMessage, Role, Transformer};

/// Drops assistant messages that carry `reasoning_details` but have neither
/// text content nor tool calls.
///
/// Anthropic rejects assistant messages whose final content block is
/// `thinking` with `HTTP 400: The final block in an assistant message cannot
/// be \`thinking\``. The only way such a message can reach the wire is when
/// the turn captured reasoning but no follow-up action — typically an
/// aborted tool call, a compaction artifact, or a streaming disconnect that
/// saved the thought but not the response. The orphaned thought cannot
/// anchor a next turn (there is nothing to anchor to) and its signature
/// wouldn't survive being grafted onto a neighbouring turn, so the least
/// lossy valid shape is to drop the whole message.
///
/// Gated at the orchestrator on Claude model ids — OpenAI-compatible paths
/// serialize reasoning as a sibling field instead of a content block and
/// don't hit the same rejection.
pub(crate) struct DropReasoningOnlyMessages;

impl Transformer for DropReasoningOnlyMessages {
    type Value = Context;

    fn transform(&mut self, mut context: Self::Value) -> Self::Value {
        context
            .messages
            .retain(|entry| !is_reasoning_only(&entry.message));
        context
    }
}

fn is_reasoning_only(message: &ContextMessage) -> bool {
    let ContextMessage::Text(msg) = message else {
        return false;
    };
    if msg.role != Role::Assistant {
        return false;
    }
    let has_text = !msg.content.is_empty();
    let has_tool_calls = msg.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty());
    let has_reasoning = msg.reasoning_details.is_some();
    has_reasoning && !has_text && !has_tool_calls
}

#[cfg(test)]
mod tests {
    use forge_domain::{
        Context, ContextMessage, ReasoningFull, Role, TextMessage, ToolCallArguments, ToolCallFull,
        ToolCallId, Transformer,
    };
    use pretty_assertions::assert_eq;

    use super::*;

    fn signed_reasoning() -> Vec<ReasoningFull> {
        vec![ReasoningFull {
            text: Some("let me think".to_string()),
            signature: Some("sig_abc".to_string()),
            ..Default::default()
        }]
    }

    #[test]
    fn test_drops_reasoning_only_assistant_message() {
        let fixture = Context::default().add_message(ContextMessage::Text(
            TextMessage::new(Role::Assistant, "").reasoning_details(signed_reasoning()),
        ));

        let actual = DropReasoningOnlyMessages.transform(fixture);

        assert!(actual.messages.is_empty());
    }

    #[test]
    fn test_keeps_assistant_message_with_text() {
        let fixture = Context::default().add_message(ContextMessage::Text(
            TextMessage::new(Role::Assistant, "hello").reasoning_details(signed_reasoning()),
        ));

        let actual = DropReasoningOnlyMessages.transform(fixture);

        assert_eq!(actual.messages.len(), 1);
    }

    #[test]
    fn test_keeps_assistant_message_with_tool_call() {
        let tool_call = ToolCallFull::new("demo")
            .call_id(ToolCallId::new("call_1"))
            .arguments(ToolCallArguments::from_json("{}"));
        let fixture = Context::default().add_message(ContextMessage::Text(
            TextMessage::new(Role::Assistant, "")
                .tool_calls(vec![tool_call])
                .reasoning_details(signed_reasoning()),
        ));

        let actual = DropReasoningOnlyMessages.transform(fixture);

        assert_eq!(actual.messages.len(), 1);
    }

    #[test]
    fn test_drops_when_tool_calls_is_empty_vec() {
        // `Some(vec![])` is semantically "no tool calls" — treat like `None`.
        let fixture = Context::default().add_message(ContextMessage::Text(
            TextMessage::new(Role::Assistant, "")
                .tool_calls(Vec::<ToolCallFull>::new())
                .reasoning_details(signed_reasoning()),
        ));

        let actual = DropReasoningOnlyMessages.transform(fixture);

        assert!(actual.messages.is_empty());
    }

    #[test]
    fn test_leaves_user_messages_untouched() {
        let fixture = Context::default()
            .add_message(ContextMessage::Text(TextMessage::new(Role::User, "hi")));

        let actual = DropReasoningOnlyMessages.transform(fixture);

        assert_eq!(actual.messages.len(), 1);
    }

    #[test]
    fn test_leaves_assistant_without_reasoning_untouched() {
        // Assistant message with empty content/tool_calls but no reasoning is
        // not this transform's concern (caller decides whether to prune).
        let fixture = Context::default()
            .add_message(ContextMessage::Text(TextMessage::new(Role::Assistant, "")));

        let actual = DropReasoningOnlyMessages.transform(fixture);

        assert_eq!(actual.messages.len(), 1);
    }
}
