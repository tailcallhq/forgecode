use derive_setters::Setters;
use serde::{Deserialize, Serialize};

use crate::{ConversationId, Event};

/// A request originating from a single user prompt in the UI.
#[derive(Debug, Serialize, Deserialize, Clone, Setters)]
#[setters(into, strip_option)]
pub struct ChatRequest {
    pub event: Event,
    pub conversation_id: ConversationId,
}

impl ChatRequest {
    /// Create a new `ChatRequest`.
    pub fn new(content: Event, conversation_id: ConversationId) -> Self {
        Self { event: content, conversation_id }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_chat_request_new() {
        let event = Event::new("hello");
        let conversation_id = ConversationId::generate();

        let req = ChatRequest::new(event, conversation_id);

        assert_eq!(req.conversation_id, conversation_id);
    }
}
