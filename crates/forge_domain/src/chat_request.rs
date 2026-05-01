use derive_setters::Setters;
use serde::{Deserialize, Serialize};

use crate::{ConversationId, Event};

#[derive(Debug, Serialize, Deserialize, Clone, Setters)]
#[setters(into, strip_option)]
pub struct ChatRequest {
    pub event: Event,
    pub conversation_id: ConversationId,
}

impl ChatRequest {
    pub fn new(content: Event, conversation_id: ConversationId) -> Self {
        Self { event: content, conversation_id }
    }
}
