//! Sandbox hook that monitors shell-related tool calls.
//!
//! After every shell/bash/sh/powershell/cmd tool call, this hook logs sandbox
//! activity so operators can audit which tool invocations executed in a
//! sandboxed context.

use async_trait::async_trait;
use forge_domain::{Conversation, EventData, EventHandle, ToolcallEndPayload};
use tracing::debug;

/// Hook that monitors shell-related tool calls for sandbox observability.
///
/// When enabled, this hook fires on every [`ToolcallEndPayload`] event and
/// logs a debug message if the tool is a shell-type command (shell, bash,
/// sh, powershell, cmd).  This provides an audit trail for which tool
/// invocations were sandboxed.
#[derive(Clone)]
pub struct SandboxHook {
    /// Whether the hook is active.
    enabled: bool,
}

impl SandboxHook {
    /// Creates a new, enabled sandbox hook.
    pub fn new() -> Self {
        Self { enabled: true }
    }

    /// Creates a disabled sandbox hook (no-op).
    #[cfg(test)]
    pub fn disabled() -> Self {
        Self { enabled: false }
    }

    /// Returns `true` if the tool name corresponds to a shell-type tool.
    pub fn is_shell_tool(tool_name: &str) -> bool {
        matches!(
            tool_name.to_lowercase().as_str(),
            "shell" | "bash" | "sh" | "powershell" | "cmd"
        )
    }
}

impl Default for SandboxHook {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventHandle<EventData<ToolcallEndPayload>> for SandboxHook {
    async fn handle(
        &self,
        event: &EventData<ToolcallEndPayload>,
        _conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let tool_call = &event.payload.tool_call;
        let result = &event.payload.result;

        if !Self::is_shell_tool(tool_call.name.as_str()) {
            return Ok(());
        }

        let status = if result.is_error() {
            "error"
        } else {
            "success"
        };

        debug!(
            tool = %tool_call.name,
            status = status,
            "Sandbox hook: shell tool call completed"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_domain::{
        Agent, AgentId, Context, ContextMessage, Conversation, ModelId, ProviderId,
        ToolCallArguments, ToolCallFull, ToolCallId, ToolName, ToolOutput, ToolResult,
    };

    fn make_toolcall_end_event(tool_name: &str, is_error: bool) -> EventData<ToolcallEndPayload> {
        let tool_call = ToolCallFull {
            name: ToolName::new(tool_name),
            call_id: Some(ToolCallId::new("test_call_id")),
            arguments: ToolCallArguments::default(),
            thought_signature: None,
        };

        let result = if is_error {
            ToolResult::new(tool_name).output(Err(anyhow::anyhow!("test error")))
        } else {
            ToolResult::new(tool_name).output(Ok(ToolOutput::text("success")))
        };

        EventData::new(
            Agent::new(
                AgentId::new("test"),
                ProviderId::OPENAI,
                ModelId::new("test-model"),
            ),
            ModelId::new("test-model"),
            ToolcallEndPayload::new(tool_call, result),
        )
    }

    fn make_conversation_with_message() -> Conversation {
        Conversation::generate()
            .context(Context::default().add_message(ContextMessage::user("test message", None)))
    }

    #[test]
    fn test_is_shell_tool_shell() {
        assert!(SandboxHook::is_shell_tool("shell"));
        assert!(SandboxHook::is_shell_tool("Shell"));
        assert!(SandboxHook::is_shell_tool("SHELL"));
    }

    #[test]
    fn test_is_shell_tool_bash() {
        assert!(SandboxHook::is_shell_tool("bash"));
        assert!(SandboxHook::is_shell_tool("Bash"));
    }

    #[test]
    fn test_is_shell_tool_powershell() {
        assert!(SandboxHook::is_shell_tool("powershell"));
        assert!(SandboxHook::is_shell_tool("PowerShell"));
    }

    #[test]
    fn test_is_shell_tool_non_shell() {
        assert!(!SandboxHook::is_shell_tool("read"));
        assert!(!SandboxHook::is_shell_tool("write"));
        assert!(!SandboxHook::is_shell_tool("patch"));
        assert!(!SandboxHook::is_shell_tool("fs_search"));
    }

    #[tokio::test]
    async fn test_sandbox_hook_triggers_on_shell() {
        let hook = SandboxHook::new();
        let mut conversation = make_conversation_with_message();

        let event = make_toolcall_end_event("shell", false);
        hook.handle(&event, &mut conversation).await.unwrap();

        // Hook should not modify the conversation — it only logs.
        let context = conversation.context.unwrap();
        // The original message should still be there, unmodified.
        assert!(!context.messages.is_empty());
    }

    #[tokio::test]
    async fn test_sandbox_hook_skips_read_tool() {
        let hook = SandboxHook::new();
        let mut conversation = Conversation::generate().context(Context::default());

        let event = make_toolcall_end_event("read", false);
        hook.handle(&event, &mut conversation).await.unwrap();

        let context = conversation.context.unwrap();
        assert!(context.messages.is_empty());
    }

    #[tokio::test]
    async fn test_sandbox_hook_disabled() {
        let hook = SandboxHook::disabled();
        let mut conversation = make_conversation_with_message();

        let event = make_toolcall_end_event("shell", false);
        hook.handle(&event, &mut conversation).await.unwrap();

        // No modification expected.
        let context = conversation.context.unwrap();
        assert_eq!(context.messages.len(), 1);
    }

    #[tokio::test]
    async fn test_sandbox_hook_logs_error_status() {
        let hook = SandboxHook::new();
        let mut conversation = make_conversation_with_message();

        // Even on error, the hook should not panic.
        let event = make_toolcall_end_event("bash", true);
        hook.handle(&event, &mut conversation).await.unwrap();

        let context = conversation.context.unwrap();
        assert!(!context.messages.is_empty());
    }
}
