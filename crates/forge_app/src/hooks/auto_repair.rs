use async_trait::async_trait;
use forge_domain::{ContextMessage, Conversation, EventData, EventHandle, ToolcallEndPayload};
use tracing::debug;

/// Hook that prompts the agent to run tests and lint after file-modifying tool
/// calls.
///
/// After every successful Write, Patch, or MultiPatch tool call, this hook
/// injects a system-level reminder into the conversation context. The reminder
/// instructs the agent to run tests and lint to verify the change, which
/// triggers an automatic repair loop if failures are found.
///
/// # Design Rationale
///
/// This hook does NOT run tests/lint itself — it delegates to the agent's
/// existing shell tool. This is intentional:
///
/// 1. **Respects agent autonomy** — the agent decides which test/lint commands
///    to run based on the project context (cargo test, npm test, etc.)
/// 2. **Handles edge cases** — the agent can skip if the change is docs-only
///    or if tests are already passing
/// 3. **Zero overhead when disabled** — the hook is a no-op if the feature is
///    not enabled
/// 4. **Non-invasive** — no changes to the orchestrator, tool execution, or
///    provider pipeline
#[derive(Clone)]
pub struct AutoRepairHook {
    /// Whether auto-repair is enabled
    enabled: bool,
    /// Optional custom test command (falls back to auto-detection)
    test_command: Option<String>,
    /// Optional custom lint command (falls back to auto-detection)
    lint_command: Option<String>,
}

impl Default for AutoRepairHook {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoRepairHook {
    /// Creates a new auto-repair hook with default settings (enabled).
    pub fn new() -> Self {
        Self { enabled: true, test_command: None, lint_command: None }
    }

    /// Creates a new auto-repair hook with custom commands.
    #[cfg(test)]
    pub fn with_commands(test_command: impl Into<String>, lint_command: impl Into<String>) -> Self {
        Self {
            enabled: true,
            test_command: Some(test_command.into()),
            lint_command: Some(lint_command.into()),
        }
    }

    /// Creates a disabled auto-repair hook.
    #[cfg(test)]
    pub fn disabled() -> Self {
        Self { enabled: false, test_command: None, lint_command: None }
    }

    /// Checks if the tool call is a file-modifying tool that should trigger
    /// auto-repair.
    fn is_file_modifying_tool(tool_name: &str) -> bool {
        matches!(
            tool_name.to_lowercase().as_str(),
            "write" | "patch" | "multi_patch" | "multipatch" | "remove"
        )
    }

    /// Builds the auto-repair reminder message to inject into context.
    fn build_repair_message(&self) -> String {
        let test_cmd = self
            .test_command
            .as_deref()
            .unwrap_or("the project's test suite");
        let lint_cmd = self
            .lint_command
            .as_deref()
            .unwrap_or("the project's linter");

        format!(
            "[Auto-Repair] You just modified a file. Please run {test_cmd} and {lint_cmd} to \
             verify the change compiles and passes all checks. If there are failures, fix them \
             before proceeding. This is an automatic quality gate — do not skip it."
        )
    }
}

#[async_trait]
impl EventHandle<EventData<ToolcallEndPayload>> for AutoRepairHook {
    async fn handle(
        &self,
        event: &EventData<ToolcallEndPayload>,
        conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let tool_call = &event.payload.tool_call;
        let result = &event.payload.result;

        // Only trigger for file-modifying tools that succeeded
        if !Self::is_file_modifying_tool(tool_call.name.as_str()) {
            return Ok(());
        }

        if result.is_error() {
            debug!(
                tool = %tool_call.name,
                "Auto-repair skipped: tool call failed"
            );
            return Ok(());
        }

        // Inject the auto-repair reminder into the conversation context
        let message_text = self.build_repair_message();

        if let Some(context) = &mut conversation.context {
            let repair_message = ContextMessage::system(message_text);
            *context = context.clone().add_message(repair_message);

            debug!(
                tool = %tool_call.name,
                "Auto-repair reminder injected into context"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_domain::{
        Agent, AgentId, Context, ModelId, ProviderId, ToolCallArguments, ToolCallFull, ToolCallId,
        ToolName, ToolOutput, ToolResult,
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

    #[tokio::test]
    async fn test_auto_repair_triggers_on_write() {
        let hook = AutoRepairHook::new();
        let mut conversation = make_conversation_with_message();

        let event = make_toolcall_end_event("write", false);
        hook.handle(&event, &mut conversation).await.unwrap();

        let context = conversation.context.unwrap();
        let last_msg = context.messages.last().unwrap();
        match &last_msg.message {
            ContextMessage::Text(text) => {
                assert!(text.content.contains("Auto-Repair"));
                assert!(text.content.contains("modified a file"));
            }
            _ => panic!("Expected text message"),
        }
    }

    #[tokio::test]
    async fn test_auto_repair_skips_read_tool() {
        let hook = AutoRepairHook::new();
        let mut conversation = Conversation::generate().context(Context::default());

        let event = make_toolcall_end_event("read", false);
        hook.handle(&event, &mut conversation).await.unwrap();

        let context = conversation.context.unwrap();
        assert!(context.messages.is_empty());
    }

    #[tokio::test]
    async fn test_auto_repair_skips_failed_tool() {
        let hook = AutoRepairHook::new();
        let mut conversation = Conversation::generate().context(Context::default());

        let event = make_toolcall_end_event("write", true);
        hook.handle(&event, &mut conversation).await.unwrap();

        let context = conversation.context.unwrap();
        assert!(context.messages.is_empty());
    }

    #[tokio::test]
    async fn test_auto_repair_disabled() {
        let hook = AutoRepairHook::disabled();
        let mut conversation = Conversation::generate().context(Context::default());

        let event = make_toolcall_end_event("write", false);
        hook.handle(&event, &mut conversation).await.unwrap();

        let context = conversation.context.unwrap();
        assert!(context.messages.is_empty());
    }

    #[tokio::test]
    async fn test_auto_repair_custom_commands() {
        let hook = AutoRepairHook::with_commands("npm test", "npm run lint");
        let mut conversation = make_conversation_with_message();

        let event = make_toolcall_end_event("patch", false);
        hook.handle(&event, &mut conversation).await.unwrap();

        let context = conversation.context.unwrap();
        let last_msg = context.messages.last().unwrap();
        match &last_msg.message {
            ContextMessage::Text(text) => {
                assert!(text.content.contains("npm test"));
                assert!(text.content.contains("npm run lint"));
            }
            _ => panic!("Expected text message"),
        }
    }

    #[tokio::test]
    async fn test_auto_repair_triggers_on_multi_patch() {
        let hook = AutoRepairHook::new();
        let mut conversation = make_conversation_with_message();

        let event = make_toolcall_end_event("multi_patch", false);
        hook.handle(&event, &mut conversation).await.unwrap();

        let context = conversation.context.unwrap();
        assert!(!context.messages.is_empty());
    }

    #[tokio::test]
    async fn test_auto_repair_triggers_on_remove() {
        let hook = AutoRepairHook::new();
        let mut conversation = make_conversation_with_message();

        let event = make_toolcall_end_event("remove", false);
        hook.handle(&event, &mut conversation).await.unwrap();

        let context = conversation.context.unwrap();
        assert!(!context.messages.is_empty());
    }
}
