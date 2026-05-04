use forge_domain::{Context, ContextMessage, Transformer};

/// Strips assistant `reasoning_details` from every message but leaves
/// `context.reasoning` (the per-turn reasoning config) intact.
///
/// Used on the z.ai provider path: when reasoning is opted out (e.g.
/// `:reasoning-effort none`), the orchestrator must keep `context.reasoning`
/// alive so [`SetZaiThinking`] can map it to `thinking: { type: disabled }`.
/// At the same time, prior assistant `reasoning_details` from earlier turns
/// must NOT replay into the request, otherwise the user's opt-out is
/// undermined and the upstream API may reject unsupported fields.
///
/// Compare to `forge_domain::DropReasoningDetails`, which clears both the
/// per-message details and `context.reasoning`.
pub(crate) struct DropMessageReasoningDetails;

impl Transformer for DropMessageReasoningDetails {
    type Value = Context;

    fn transform(&mut self, mut context: Self::Value) -> Self::Value {
        context.messages.iter_mut().for_each(|message| {
            if let ContextMessage::Text(text) = &mut **message {
                text.reasoning_details = None;
            }
        });
        context
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::{ContextMessage, ReasoningConfig, ReasoningFull, Role, TextMessage};

    use super::*;

    fn make_reasoning() -> Vec<ReasoningFull> {
        vec![ReasoningFull {
            text: Some("prior thinking".to_string()),
            signature: None,
            ..Default::default()
        }]
    }

    #[test]
    fn drops_message_reasoning_details_but_keeps_context_reasoning() {
        let reasoning = make_reasoning();
        let fixture = Context::default()
            .add_message(ContextMessage::Text(
                TextMessage::new(Role::Assistant, "hi").reasoning_details(reasoning),
            ))
            .reasoning(ReasoningConfig { enabled: Some(false), ..Default::default() });

        let mut transformer = DropMessageReasoningDetails;
        let actual = transformer.transform(fixture);

        let cleared = actual.messages.iter().all(|entry| match &**entry {
            ContextMessage::Text(t) => t.reasoning_details.is_none(),
            _ => true,
        });
        assert!(cleared, "all message reasoning_details should be cleared");
        assert!(
            actual.reasoning.is_some(),
            "context.reasoning must survive so SetZaiThinking can map it"
        );
    }
}
