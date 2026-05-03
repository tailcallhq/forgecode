use forge_domain::{ContextMessage, MessageEntry, Role};

/// Newtype around `MessageEntry` so `forge_compact::ContextMessage` can
/// be implemented here — the trait and the type live in different
/// upstream crates, which Rust's orphan rule otherwise forbids.
#[derive(Clone, Debug, PartialEq)]
pub struct CompactableEntry(pub MessageEntry);

impl CompactableEntry {
    pub fn entry(&self) -> &MessageEntry {
        &self.0
    }

    pub fn into_entry(self) -> MessageEntry {
        self.0
    }
}

impl forge_compact::ContextMessage for CompactableEntry {
    fn is_user(&self) -> bool {
        matches!(&self.0.message, ContextMessage::Text(t) if t.role == Role::User)
    }

    fn is_assistant(&self) -> bool {
        matches!(&self.0.message, ContextMessage::Text(t) if t.role == Role::Assistant)
    }

    fn is_system(&self) -> bool {
        matches!(&self.0.message, ContextMessage::Text(t) if t.role == Role::System)
    }

    fn is_toolcall(&self) -> bool {
        matches!(
            &self.0.message,
            ContextMessage::Text(t)
                if t.role == Role::Assistant
                && t.tool_calls.as_ref().is_some_and(|c| !c.is_empty())
        )
    }

    fn is_toolcall_result(&self) -> bool {
        matches!(&self.0.message, ContextMessage::Tool(_))
    }
}

#[cfg(test)]
mod tests {
    use forge_compact::ContextMessage as _;
    use forge_domain::{
        ContextMessage, Image, MessageEntry, Role, TextMessage, ToolCallFull, ToolCallId, ToolName,
        ToolOutput, ToolResult,
    };

    use super::*;

    fn wrap(msg: ContextMessage) -> CompactableEntry {
        CompactableEntry(MessageEntry::from(msg))
    }

    /// Each role returns `true` for exactly one of the role-check methods.
    #[test]
    fn test_role_discriminators_are_mutually_exclusive() {
        let u = wrap(ContextMessage::Text(TextMessage::new(Role::User, "q")));
        assert!(u.is_user());
        assert!(!u.is_assistant());
        assert!(!u.is_system());

        let a = wrap(ContextMessage::Text(TextMessage::new(Role::Assistant, "r")));
        assert!(a.is_assistant());
        assert!(!a.is_user());
        assert!(!a.is_system());

        let s = wrap(ContextMessage::Text(TextMessage::new(Role::System, "sys")));
        assert!(s.is_system());
        assert!(!s.is_user());
        assert!(!s.is_assistant());
    }

    /// An assistant text message with no tool calls is not a toolcall.
    #[test]
    fn test_plain_assistant_is_not_a_toolcall() {
        let a = wrap(ContextMessage::Text(TextMessage::new(Role::Assistant, "r")));
        assert!(!a.is_toolcall());
    }

    /// An assistant message carrying at least one `ToolCallFull` is a toolcall.
    #[test]
    fn test_assistant_with_tool_calls_is_a_toolcall() {
        let call = ToolCallFull::new(ToolName::new("read")).call_id("c1");
        let a = wrap(ContextMessage::Text(
            TextMessage::new(Role::Assistant, "r").tool_calls(vec![call]),
        ));
        assert!(a.is_toolcall());
        assert!(a.is_assistant());
    }

    /// `ContextMessage::Tool` maps to `is_toolcall_result`.
    #[test]
    fn test_tool_variant_is_toolcall_result() {
        let r = wrap(ContextMessage::Tool(ToolResult {
            name: ToolName::new("read"),
            call_id: Some(ToolCallId::new("c1")),
            output: ToolOutput::text("ok"),
        }));
        assert!(r.is_toolcall_result());
        assert!(!r.is_user());
        assert!(!r.is_assistant());
        assert!(!r.is_system());
        assert!(!r.is_toolcall());
    }

    /// Images are neither role-shaped nor toolcall-shaped; every check
    /// returns false so the compaction algorithm passes them through.
    #[test]
    fn test_image_returns_false_for_every_predicate() {
        let i = wrap(ContextMessage::Image(Image::new_base64(
            "aGVsbG8=".to_string(),
            "image/png",
        )));
        assert!(!i.is_user());
        assert!(!i.is_assistant());
        assert!(!i.is_system());
        assert!(!i.is_toolcall());
        assert!(!i.is_toolcall_result());
    }
}
