use crate::{ContextMessage, MessageEntry};

/// In-flight turn content that lives only in request-build scope. Never
/// persisted to `conversations.context` until the turn completes; halted
/// turns discard it and leave canonical byte-identical.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PendingTurn {
    /// User's input for this turn — primary user message, piped additional
    /// context, on-resume todo reminders, attachment blocks, and any other
    /// messages injected alongside the user's prompt.
    pub user_input: Vec<MessageEntry>,

    /// In-flight content accumulated during the tool-call loop: assistant
    /// messages carrying `tool_use` blocks, and the `tool_result` messages
    /// their tools produced. Empty on the first loop iteration and grows
    /// only while the turn is in flight. v2 microcompact will target
    /// `tool_result` blocks here; v1 handles them in bulk.
    pub continuation: Vec<MessageEntry>,
}

impl PendingTurn {
    pub fn is_empty(&self) -> bool {
        self.user_input.is_empty() && self.continuation.is_empty()
    }

    pub fn is_continuation(&self) -> bool {
        !self.continuation.is_empty()
    }

    pub fn push_user_input(&mut self, message: ContextMessage) {
        self.user_input.push(MessageEntry::from(message));
    }

    pub fn push_continuation(&mut self, message: ContextMessage) {
        self.continuation.push(MessageEntry::from(message));
    }

    pub fn iter_messages(&self) -> impl Iterator<Item = &MessageEntry> {
        self.user_input.iter().chain(self.continuation.iter())
    }

    pub fn into_messages(self) -> Vec<MessageEntry> {
        let mut out = self.user_input;
        out.extend(self.continuation);
        out
    }

    /// Character-based token approximation across all pending messages.
    pub fn token_count_approx(&self) -> usize {
        self.iter_messages().map(|m| m.token_count_approx()).sum()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::TextMessage;

    fn user(text: &str) -> ContextMessage {
        ContextMessage::Text(TextMessage::new(crate::Role::User, text))
    }

    fn assistant(text: &str) -> ContextMessage {
        ContextMessage::Text(TextMessage::new(crate::Role::Assistant, text))
    }

    /// An empty pending reports empty, non-continuation, and iterates to
    /// nothing.
    #[test]
    fn test_default_pending_is_empty() {
        let p = PendingTurn::default();
        assert!(p.is_empty());
        assert!(!p.is_continuation());
        assert_eq!(p.iter_messages().count(), 0);
    }

    /// Only `continuation` flips the `is_continuation()` flag — adding
    /// user input doesn't by itself signal a tool-call continuation.
    #[test]
    fn test_is_continuation_tracks_continuation_slot_only() {
        let mut p = PendingTurn::default();
        p.push_user_input(user("hi"));
        assert!(!p.is_continuation());

        p.push_continuation(assistant("calling"));
        assert!(p.is_continuation());
    }

    /// `iter_messages` yields `user_input` first, `continuation` second,
    /// in stable order. `into_messages` flattens the same way.
    #[test]
    fn test_message_ordering_is_input_then_continuation() {
        let mut p = PendingTurn::default();
        p.push_user_input(user("u1"));
        p.push_user_input(user("u2"));
        p.push_continuation(assistant("a1"));
        p.push_continuation(assistant("a2"));

        let by_ref: Vec<_> = p
            .iter_messages()
            .filter_map(|m| m.message.content())
            .collect();
        assert_eq!(by_ref, vec!["u1", "u2", "a1", "a2"]);

        let flattened: Vec<_> = p
            .into_messages()
            .into_iter()
            .filter_map(|m| m.message.content().map(str::to_string))
            .collect();
        assert_eq!(flattened, vec!["u1", "u2", "a1", "a2"]);
    }
}
