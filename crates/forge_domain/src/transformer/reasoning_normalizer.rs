use crate::{Context, ModelId, Transformer};

/// A transformer that preserves reasoning only for the contiguous tail of
/// assistant messages that were produced by the current model, stripping
/// reasoning from everything before the first model mismatch (going backwards).
///
/// # Behaviour
///
/// Walk backwards through the assistant messages. As long as each message's
/// model matches `model_id`, its reasoning is kept. The moment a message with
/// a different model is encountered that index becomes the *cutoff*: reasoning
/// is stripped from that message and every assistant message before it,
/// regardless of which model produced them.
///
/// For example, given the assistant-message sequence `[1 1 1 1 2 1 3 1 2 2 2]`
/// with current model `2`:
/// - Tail `[2 2 2]` is preserved.
/// - Everything at or before the `1` that precedes the tail is stripped,
///   including the earlier `2` that appears before the model break.
///
/// When every assistant message was produced by the current model the
/// transformer is a no-op.  When there are no assistant messages it is also a
/// no-op.
///
/// NOTE: `context.reasoning` (the config) is never removed so the new request
/// can still enable reasoning on the current turn.
pub struct ReasoningNormalizer {
    model_id: ModelId,
}

impl ReasoningNormalizer {
    /// Creates a normalizer for the given current model.
    pub fn new(model_id: ModelId) -> Self {
        Self { model_id }
    }
}

impl Transformer for ReasoningNormalizer {
    type Value = Context;

    fn transform(&mut self, mut context: Self::Value) -> Self::Value {
        // Walk backwards to find the last assistant message (forward index) whose
        // model differs from the current one.  That is the cutoff: everything at
        // or before it has reasoning stripped; the same-model tail after it is
        // kept intact.
        let cutoff = context
            .messages
            .iter()
            .enumerate()
            .rev()
            .find_map(|(idx, msg)| {
                if msg.has_role(crate::Role::Assistant)
                    && let crate::ContextMessage::Text(text) = &**msg
                    && text.model.as_ref() != Some(&self.model_id)
                {
                    return Some(idx);
                }
                None
            });

        let Some(cutoff) = cutoff else {
            return context; // all assistant messages match — nothing to strip
        };

        for (idx, message) in context.messages.iter_mut().enumerate() {
            if idx > cutoff {
                break;
            }
            if message.has_role(crate::Role::Assistant)
                && let crate::ContextMessage::Text(text_msg) = &mut **message
            {
                text_msg.reasoning_details = None;
                text_msg.thought_signature = None;
            }
        }

        context
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_yaml_snapshot;
    use serde::Serialize;

    use super::*;
    use crate::{ContextMessage, ReasoningConfig, ReasoningFull, Role, TextMessage};

    #[derive(Serialize)]
    struct TransformationSnapshot {
        transformation: String,
        before: Context,
        after: Context,
    }

    impl TransformationSnapshot {
        fn new(transformation: &str, before: Context, after: Context) -> Self {
            Self { transformation: transformation.to_string(), before, after }
        }
    }

    fn model_a() -> ModelId {
        ModelId::from("model-a")
    }

    fn model_b() -> ModelId {
        ModelId::from("model-b")
    }

    fn model_c() -> ModelId {
        ModelId::from("model-c")
    }

    fn reasoning_details() -> Vec<ReasoningFull> {
        vec![ReasoningFull {
            text: Some("I need to think about this carefully".to_string()),
            signature: Some("sig_model_a".to_string()),
            ..Default::default()
        }]
    }

    /// Shorthand for an assistant `ContextMessage` with model and reasoning
    /// set.
    fn assistant_msg(model: ModelId, content: &str) -> ContextMessage {
        ContextMessage::Text(
            TextMessage::new(Role::Assistant, content)
                .model(model)
                .reasoning_details(reasoning_details()),
        )
    }

    /// Builds a context where the last assistant message was produced by
    /// `prev_model`.
    fn fixture_with_prev_model(prev_model: ModelId) -> Context {
        Context::default()
            .reasoning(ReasoningConfig::default().enabled(true))
            .add_message(ContextMessage::user("First question", None))
            .add_message(assistant_msg(
                prev_model.clone(),
                "First assistant response",
            ))
            .add_message(ContextMessage::user("Follow-up question", None))
            .add_message(assistant_msg(prev_model, "Second assistant response"))
    }

    #[test]
    fn test_no_op_when_model_unchanged() {
        // When the current model matches the last assistant message's model,
        // the transformer must not touch any reasoning details.
        let fixture = fixture_with_prev_model(model_a());
        let mut transformer = ReasoningNormalizer::new(model_a());
        let actual = transformer.transform(fixture.clone());

        assert_eq!(
            actual, fixture,
            "Context should be unchanged when model is the same"
        );
    }

    #[test]
    fn test_strips_all_reasoning_when_model_changed() {
        // When the model changes, ALL reasoning must be stripped (including the
        // last assistant message) because signatures from the old model are
        // invalid for the new model.
        let fixture = fixture_with_prev_model(model_a());
        let mut transformer = ReasoningNormalizer::new(model_b());
        let actual = transformer.transform(fixture);

        for message in &actual.messages {
            if message.has_role(Role::Assistant)
                && let crate::ContextMessage::Text(text) = &**message
            {
                assert_eq!(
                    text.reasoning_details, None,
                    "All assistant reasoning must be stripped on model change"
                );
            }
        }
    }

    #[test]
    fn test_reasoning_config_preserved_when_model_changed() {
        // Stripping reasoning blocks must not disable the reasoning config,
        // so the new model can still reason on the current turn.
        let fixture = fixture_with_prev_model(model_a());
        let mut transformer = ReasoningNormalizer::new(model_b());
        let actual = transformer.transform(fixture);

        assert!(
            actual.reasoning.is_some(),
            "Reasoning config must be preserved so new model can still reason"
        );
        assert_eq!(actual.reasoning.as_ref().unwrap().enabled, Some(true));
    }

    #[test]
    fn test_no_op_when_no_previous_assistant_message() {
        // No previous assistant message means no previous model to compare
        // against — treat as unchanged (no-op).
        let fixture = Context::default()
            .reasoning(ReasoningConfig::default().enabled(true))
            .add_message(ContextMessage::system("System message"))
            .add_message(ContextMessage::user("User message", None));

        let mut transformer = ReasoningNormalizer::new(model_b());
        let actual = transformer.transform(fixture.clone());

        assert_eq!(
            actual, fixture,
            "Context should be unchanged when there is no previous assistant"
        );
    }

    // --- Back-and-forth model change tests ---

    #[test]
    fn test_a_to_b_strips_reasoning() {
        // A → B: switching from model_a to model_b must strip all reasoning.
        let fixture = Context::default()
            .reasoning(ReasoningConfig::default().enabled(true))
            .add_message(ContextMessage::user("q1", None))
            .add_message(assistant_msg(model_a(), "a1"))
            .add_message(ContextMessage::user("q2", None))
            .add_message(assistant_msg(model_a(), "a2")); // last assistant is model_a

        let actual = ReasoningNormalizer::new(model_b()).transform(fixture);

        assert!(all_reasoning_stripped(&actual));
    }

    #[test]
    fn test_a_to_b_back_to_a_strips_reasoning() {
        // A → B → A: switching back to model_a after model_b must still strip,
        // because the last assistant message carries model_b signatures.
        let fixture = Context::default()
            .reasoning(ReasoningConfig::default().enabled(true))
            .add_message(ContextMessage::user("q1", None))
            .add_message(assistant_msg(model_a(), "a1"))
            .add_message(ContextMessage::user("q2", None))
            .add_message(assistant_msg(model_b(), "a2")) // last assistant is model_b
            .add_message(ContextMessage::user("q3", None));

        let actual = ReasoningNormalizer::new(model_a()).transform(fixture);

        assert!(all_reasoning_stripped(&actual));
    }

    #[test]
    fn test_a_to_b_stay_on_b_strips_a_keeps_b() {
        // A → B (stay on B): the model_a message before the switch loses its
        // reasoning (it's before the cutoff); the model_b tail is preserved.
        let fixture = Context::default()
            .reasoning(ReasoningConfig::default().enabled(true))
            .add_message(ContextMessage::user("q1", None))
            .add_message(assistant_msg(model_a(), "a1"))
            .add_message(ContextMessage::user("q2", None))
            .add_message(assistant_msg(model_b(), "a2")); // last assistant is model_b

        let actual = ReasoningNormalizer::new(model_b()).transform(fixture);

        // a1 (model_a) is before the cutoff → stripped
        let msgs: Vec<_> = actual.messages.iter().collect();
        if let crate::ContextMessage::Text(a1) = &**msgs[1] {
            assert_eq!(
                a1.reasoning_details, None,
                "a1 (model_a) should be stripped"
            );
        }
        // a2 (model_b) is in the same-model tail → preserved
        if let crate::ContextMessage::Text(a2) = &**msgs[3] {
            assert_eq!(
                a2.reasoning_details,
                Some(reasoning_details()),
                "a2 (model_b) should be preserved"
            );
        }
    }

    #[test]
    fn test_a_to_b_to_c_strips_reasoning() {
        // A → B → C: every model switch must strip; here B→C triggers the strip.
        let fixture = Context::default()
            .reasoning(ReasoningConfig::default().enabled(true))
            .add_message(ContextMessage::user("q1", None))
            .add_message(assistant_msg(model_a(), "a1"))
            .add_message(ContextMessage::user("q2", None))
            .add_message(assistant_msg(model_b(), "a2")); // last assistant is model_b

        let actual = ReasoningNormalizer::new(model_c()).transform(fixture);

        assert!(all_reasoning_stripped(&actual));
    }

    #[test]
    fn test_alternating_a_b_a_b_strips_reasoning() {
        // A → B → A → B: after the full alternation the last assistant is model_a;
        // switching to model_b must strip all reasoning.
        let fixture = Context::default()
            .reasoning(ReasoningConfig::default().enabled(true))
            .add_message(ContextMessage::user("q1", None))
            .add_message(assistant_msg(model_a(), "a1"))
            .add_message(ContextMessage::user("q2", None))
            .add_message(assistant_msg(model_b(), "a2"))
            .add_message(ContextMessage::user("q3", None))
            .add_message(assistant_msg(model_a(), "a3")); // last assistant is model_a

        let actual = ReasoningNormalizer::new(model_b()).transform(fixture);

        assert!(all_reasoning_stripped(&actual));
    }

    #[test]
    fn test_alternating_a_b_a_stay_a_strips_ab_keeps_last_a() {
        // A → B → A (stay on A): the cutoff is at b2 (the first mismatch going
        // backwards), so a1 and b2 lose reasoning; only a3 (the same-model tail)
        // is preserved.
        let fixture = Context::default()
            .reasoning(ReasoningConfig::default().enabled(true))
            .add_message(ContextMessage::user("q1", None))
            .add_message(assistant_msg(model_a(), "a1"))
            .add_message(ContextMessage::user("q2", None))
            .add_message(assistant_msg(model_b(), "b2"))
            .add_message(ContextMessage::user("q3", None))
            .add_message(assistant_msg(model_a(), "a3")); // last assistant is model_a

        let actual = ReasoningNormalizer::new(model_a()).transform(fixture);

        let msgs: Vec<_> = actual.messages.iter().collect();
        // a1 (model_a, before cutoff) → stripped
        if let crate::ContextMessage::Text(a1) = &**msgs[1] {
            assert_eq!(
                a1.reasoning_details, None,
                "a1 should be stripped (before cutoff)"
            );
        }
        // b2 (model_b, the cutoff itself) → stripped
        if let crate::ContextMessage::Text(b2) = &**msgs[3] {
            assert_eq!(
                b2.reasoning_details, None,
                "b2 should be stripped (is the cutoff)"
            );
        }
        // a3 (model_a, same-model tail) → preserved
        if let crate::ContextMessage::Text(a3) = &**msgs[5] {
            assert_eq!(
                a3.reasoning_details,
                Some(reasoning_details()),
                "a3 should be preserved (same-model tail)"
            );
        }
    }

    /// Returns `true` when every assistant message in `ctx` has no reasoning
    /// details.
    fn all_reasoning_stripped(ctx: &Context) -> bool {
        ctx.messages.iter().all(|msg| {
            if msg.has_role(Role::Assistant)
                && let crate::ContextMessage::Text(text) = &**msg
            {
                return text.reasoning_details.is_none();
            }
            true
        })
    }

    #[test]
    fn test_mixed_sequence_preserves_only_same_model_tail() {
        // Sequence: a a a a b a c a b b b  (current = b)
        //                             ↑↑↑  preserved tail
        //           ↑↑↑↑↑↑↑↑↑↑↑      stripped (everything before the tail break)
        // The earlier `b` in the middle is also stripped because it is before
        // the cutoff — only the contiguous tail from the end matters.
        let fixture = Context::default()
            .reasoning(ReasoningConfig::default().enabled(true))
            .add_message(ContextMessage::user("q1", None))
            .add_message(assistant_msg(model_a(), "a1"))
            .add_message(ContextMessage::user("q2", None))
            .add_message(assistant_msg(model_a(), "a2"))
            .add_message(ContextMessage::user("q3", None))
            .add_message(assistant_msg(model_a(), "a3"))
            .add_message(ContextMessage::user("q4", None))
            .add_message(assistant_msg(model_a(), "a4"))
            .add_message(ContextMessage::user("q5", None))
            .add_message(assistant_msg(model_b(), "b5")) // earlier b — must be stripped
            .add_message(ContextMessage::user("q6", None))
            .add_message(assistant_msg(model_a(), "a6"))
            .add_message(ContextMessage::user("q7", None))
            .add_message(assistant_msg(model_c(), "c7"))
            .add_message(ContextMessage::user("q8", None))
            .add_message(assistant_msg(model_a(), "a8"))
            .add_message(ContextMessage::user("q9", None))
            .add_message(assistant_msg(model_b(), "b9")) // tail start
            .add_message(ContextMessage::user("q10", None))
            .add_message(assistant_msg(model_b(), "b10")) // tail
            .add_message(ContextMessage::user("q11", None))
            .add_message(assistant_msg(model_b(), "b11")); // tail end (last)

        let actual = ReasoningNormalizer::new(model_b()).transform(fixture);

        let assistant_msgs: Vec<_> = actual
            .messages
            .iter()
            .filter(|m| m.has_role(Role::Assistant))
            .collect();

        // Tail (last 3): b9, b10, b11 → reasoning preserved
        for tail_msg in &assistant_msgs[assistant_msgs.len() - 3..] {
            if let crate::ContextMessage::Text(t) = &***tail_msg {
                assert_eq!(
                    t.reasoning_details,
                    Some(reasoning_details()),
                    "tail model_b message should preserve reasoning: {}",
                    t.content
                );
            }
        }

        // Everything before the tail (a1..a8, b5, c7) → reasoning stripped
        for pre_msg in &assistant_msgs[..assistant_msgs.len() - 3] {
            if let crate::ContextMessage::Text(t) = &***pre_msg {
                assert_eq!(
                    t.reasoning_details, None,
                    "pre-tail message should have reasoning stripped: {}",
                    t.content
                );
            }
        }

        // Reasoning config must still be enabled
        assert_eq!(actual.reasoning.as_ref().unwrap().enabled, Some(true));
    }

    #[test]
    fn test_model_changed_snapshot() {
        let fixture = fixture_with_prev_model(model_a());
        let mut transformer = ReasoningNormalizer::new(model_b());
        let actual = transformer.transform(fixture.clone());

        let snapshot =
            TransformationSnapshot::new("ReasoningNormalizer_model_changed", fixture, actual);
        assert_yaml_snapshot!(snapshot);
    }

    #[test]
    fn test_model_unchanged_snapshot() {
        let fixture = fixture_with_prev_model(model_a());
        let mut transformer = ReasoningNormalizer::new(model_a());
        let actual = transformer.transform(fixture.clone());

        let snapshot =
            TransformationSnapshot::new("ReasoningNormalizer_model_unchanged", fixture, actual);
        assert_yaml_snapshot!(snapshot);
    }
}
