use derive_setters::Setters;
use serde::{Deserialize, Serialize};

use crate::{ConversationId, Event};

#[derive(Debug, Serialize, Deserialize, Clone, Setters)]
#[setters(into, strip_option)]
pub struct ChatRequest {
    pub event: Event,
    pub conversation_id: ConversationId,
    /// When `true`, shell tool output is suppressed on stdout (routed to
    /// `io::sink()`) to protect the ACP JSON-RPC transport.
    /// See `designs/acp-silent-mode-propagation.md`.
    pub tool_silent: bool,
}

impl ChatRequest {
    pub fn new(content: Event, conversation_id: ConversationId) -> Self {
        Self { event: content, conversation_id, tool_silent: false }
    }
}
