use std::fmt::{Display, Formatter};

use derive_setters::Setters;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ConversationId, Event};

/// A unique identifier for a single user-initiated input from the UI.
///
/// Generated once per incoming `ChatRequest` at the API boundary. Every file
/// snapshot taken while processing that request is stamped with this ID,
/// allowing all changes from a single prompt to be grouped and undone together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserInputId(Uuid);

impl UserInputId {
    /// Generate a new random `UserInputId`.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Return the underlying UUID.
    pub fn into_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for UserInputId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for UserInputId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A request originating from a single user prompt in the UI.
#[derive(Debug, Serialize, Deserialize, Clone, Setters)]
#[setters(into, strip_option)]
pub struct ChatRequest {
    /// Unique identifier for this user input. Stamped on every file snapshot
    /// taken while processing this request, enabling prompt-level undo.
    pub id: UserInputId,
    pub event: Event,
    pub conversation_id: ConversationId,
}

impl ChatRequest {
    /// Create a new `ChatRequest`, generating a fresh `UserInputId`.
    pub fn new(content: Event, conversation_id: ConversationId) -> Self {
        Self { id: UserInputId::new(), event: content, conversation_id }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_user_input_id_is_unique() {
        let id1 = UserInputId::new();
        let id2 = UserInputId::new();

        assert_ne!(id1, id2);
    }

    #[test]
    fn test_user_input_id_serde_round_trip() {
        let fixture = UserInputId::new();

        let serialized = serde_json::to_string(&fixture).unwrap();
        let actual: UserInputId = serde_json::from_str(&serialized).unwrap();

        assert_eq!(actual, fixture);
    }

    #[test]
    fn test_chat_request_new_generates_unique_ids() {
        let event = Event::new("hello");
        let conversation_id = ConversationId::generate();

        let req1 = ChatRequest::new(event.clone(), conversation_id);
        let req2 = ChatRequest::new(event.clone(), conversation_id);

        assert_ne!(req1.id, req2.id);
    }
}
