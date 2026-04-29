use std::path::PathBuf;

use derive_setters::Setters;
use serde::{Deserialize, Serialize};

use crate::{ConversationId, Event};

#[derive(Debug, Serialize, Deserialize, Clone, Setters)]
#[setters(into, strip_option)]
pub struct ChatRequest {
    pub event: Event,
    pub conversation_id: ConversationId,
    /// Optional working directory override for this chat request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd_override: Option<PathBuf>,
}

impl ChatRequest {
    pub fn new(content: Event, conversation_id: ConversationId) -> Self {
        Self { event: content, conversation_id, cwd_override: None }
    }
}
