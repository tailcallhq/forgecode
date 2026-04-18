use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use forge_config::{UserHookConfig, UserHookEntry, UserHookEventName, UserHookMatcherGroup};
use forge_domain::{
    ContextMessage, Conversation, EndPayload, EventData, EventHandle, HookEventInput,
    HookExecutionResult, HookInput, HookOutput, PromptSuppressed, RequestPayload, ResponsePayload,
    Role, StartPayload, ToolCallArguments, ToolcallEndPayload, ToolcallStartPayload,
};
use regex::Regex;
use serde_json::Value;
use tracing::{debug, warn};

use super::user_hook_executor::UserHookExecutor;
use crate::services::HookCommandService;

/// Default timeout for hook commands (10 minutes).
const DEFAULT_HOOK_TIMEOUT: Duration = Duration::from_secs(600);

/// EventHandle implementation that bridges user-configured hooks with the
/// existing lifecycle event system.
///
/// This handler is constructed from a `UserHookConfig` and executes matching
/// hook commands at each lifecycle event point. It wires into the existing
/// `Hook` system via `Hook::zip()`.
#[derive(Clone)]
pub struct UserHookHandler<I> {
    executor: UserHookExecutor<I>,
    config: UserHookConfig,
    cwd: PathBuf,
    env_vars: HashMap<String, String>,
    /// Pre-compiled regex cache keyed by the raw pattern string.
    /// Built once during construction from the immutable config patterns.
    regex_cache: HashMap<String, Regex>,
}

impl<I> UserHookHandler<I> {
    /// Creates a new user hook handler from configuration.
    ///
    /// # Arguments
    /// * `service` - The hook command service used to execute hook commands.
    /// * `config` - The merged user hook configuration.
    /// * `cwd` - Current working directory for command execution.
    /// * `project_dir` - Project root directory for `FORGE_PROJECT_DIR` env
    ///   var.
    /// * `session_id` - Current session/conversation ID.
    /// * `default_hook_timeout` - Default timeout in milliseconds for hook
    ///   commands.
    pub fn new(
        service: I,
        mut env_vars: BTreeMap<String, String>,
        config: UserHookConfig,
        cwd: PathBuf,
        session_id: String,
    ) -> Self {
        env_vars.insert(
            "FORGE_PROJECT_DIR".to_string(),
            cwd.to_string_lossy().to_string(),
        );
        env_vars.insert("FORGE_SESSION_ID".to_string(), session_id);
        env_vars.insert("FORGE_CWD".to_string(), cwd.to_string_lossy().to_string());

        // Pre-compile all regex patterns from the config into a cache.
        let regex_cache = Self::build_regex_cache(&config);

        Self {
            executor: UserHookExecutor::new(service),
            config,
            cwd,
            env_vars: env_vars.into_iter().collect(),
            regex_cache,
        }
    }

    /// Pre-compiles all unique, non-empty regex patterns found in the config.
    ///
    /// Invalid patterns are logged and skipped so that construction never
    /// fails. The same warning will fire at match time for any pattern
    /// missing from the cache.
    fn build_regex_cache(config: &UserHookConfig) -> HashMap<String, Regex> {
        let mut cache = HashMap::new();
        for groups in config.events.values() {
            for group in groups {
                if let Some(pattern) = &group.matcher
                    && !pattern.is_empty()
                    && !cache.contains_key(pattern)
                {
                    match Regex::new(pattern) {
                        Ok(re) => {
                            cache.insert(pattern.clone(), re);
                        }
                        Err(e) => {
                            warn!(
                                pattern = pattern,
                                error = %e,
                                "Invalid regex in hook matcher, will be skipped at match time"
                            );
                        }
                    }
                }
            }
        }
        cache
    }

    /// Checks if the config has any hooks for the given event.
    fn has_hooks(&self, event: &UserHookEventName) -> bool {
        !self.config.get_groups(event).is_empty()
    }

    /// Constructs a [`HookInput`] from the common fields stored in this
    /// handler, leaving only the event-specific `event_data` to the caller.
    fn build_base_input(
        &self,
        event_name: &UserHookEventName,
        event_data: HookEventInput,
    ) -> HookInput {
        HookInput {
            hook_event_name: event_name.to_string(),
            cwd: self.cwd.to_string_lossy().to_string(),
            session_id: self.env_vars.get("FORGE_SESSION_ID").cloned(),
            event_data,
        }
    }

    /// Finds matching hook entries for an event, filtered by the optional
    /// matcher regex against the given subject string.
    ///
    /// Uses the pre-compiled `regex_cache` to avoid recompiling patterns on
    /// every invocation. Patterns that failed compilation during construction
    /// are silently skipped (already warned at startup).
    fn find_matching_hooks<'a>(
        groups: &'a [UserHookMatcherGroup],
        subject: Option<&str>,
        regex_cache: &HashMap<String, Regex>,
    ) -> Vec<&'a UserHookEntry> {
        let mut matching = Vec::new();

        for group in groups {
            let matches = match (&group.matcher, subject) {
                (None, _) => {
                    // No matcher means unconditional match
                    true
                }
                (Some(pattern), _) if pattern.is_empty() => {
                    // Empty matcher is treated as unconditional (same as None)
                    true
                }
                (Some(_), None) => {
                    // Matcher specified but no subject to match against; skip
                    false
                }
                (Some(pattern), Some(subj)) => {
                    regex_cache.get(pattern).is_some_and(|re| re.is_match(subj))
                }
            };

            if matches {
                matching.extend(group.hooks.iter());
            }
        }

        matching
    }

    /// Executes a list of hook entries and returns their results along with
    /// any warnings for commands that failed to execute.
    /// Each result is paired with the command string that produced it.
    async fn execute_hooks(
        &self,
        hooks: &[&UserHookEntry],
        input: &HookInput,
    ) -> (Vec<(String, HookExecutionResult)>, Vec<String>)
    where
        I: HookCommandService,
    {
        let input_json = match serde_json::to_string(input) {
            Ok(json) => json,
            Err(e) => {
                warn!(error = %e, "Failed to serialize hook input");
                return (
                    Vec::new(),
                    vec![format!("Hook input serialization failed: {e}")],
                );
            }
        };

        let mut results = Vec::new();
        let mut warnings = Vec::new();
        for hook in hooks {
            if let Some(command) = &hook.command {
                match self
                    .executor
                    .execute(
                        command,
                        &input_json,
                        hook.timeout
                            .map(Duration::from_millis)
                            .unwrap_or(DEFAULT_HOOK_TIMEOUT),
                        &self.cwd,
                        &self.env_vars,
                    )
                    .await
                {
                    Ok(result) => {
                        // Non-blocking errors (exit code 1, etc.) are warned
                        if result.is_non_blocking_error() {
                            let stderr = result.stderr.trim();
                            let detail = if stderr.is_empty() {
                                format!("exit code {:?}", result.exit_code)
                            } else {
                                stderr.to_string()
                            };
                            warnings.push(format!(
                                "Hook command '{command}' returned non-blocking error: {detail}"
                            ));
                        }
                        results.push((command.clone(), result));
                    }
                    Err(e) => {
                        warn!(
                            command = command,
                            error = %e,
                            "Hook command failed to execute"
                        );
                        warnings.push(format!("Hook command '{command}' failed to execute: {e}"));
                    }
                }
            }
        }

        (results, warnings)
    }

    /// Runs matching hooks for the given event and collects results.
    ///
    /// This encapsulates the common lifecycle hook pattern:
    /// 1. Resolve matcher groups for the event.
    /// 2. Find hooks matching the optional subject.
    /// 3. Execute matched hooks, collecting results and warnings.
    /// 4. Extend event warnings.
    /// 5. Collect and inject any `additionalContext` into the conversation.
    ///
    /// Returns the raw results for event-specific post-processing.
    async fn run_hooks_and_collect(
        &self,
        event_name: &UserHookEventName,
        subject: Option<&str>,
        input: &HookInput,
        warnings: &mut Vec<String>,
        conversation: &mut Conversation,
    ) -> Vec<(String, HookExecutionResult)>
    where
        I: HookCommandService,
    {
        let groups = self.config.get_groups(event_name);
        let hooks = Self::find_matching_hooks(groups, subject, &self.regex_cache);

        if hooks.is_empty() {
            return Vec::new();
        }

        let (results, exec_warnings) = self.execute_hooks(&hooks, input).await;
        warnings.extend(exec_warnings);

        let contexts = Self::collect_additional_context(&results);
        Self::inject_additional_context(conversation, &event_name.to_string(), &contexts);

        results
    }

    /// Checks a single hook result for blocking signals (exit code 2 or JSON
    /// blocking decision). Returns the blocking command and reason if found.
    fn check_blocking(command: &str, result: &HookExecutionResult) -> Option<(String, String)> {
        if result.is_blocking_exit() {
            let message = result
                .blocking_message()
                .unwrap_or("Hook blocked execution")
                .to_string();
            return Some((command.to_string(), message));
        }

        if let Some(output) = result.parse_output()
            && output.is_blocking()
        {
            let reason = output.blocking_reason("Hook blocked execution");
            return Some((command.to_string(), reason));
        }

        None
    }

    /// Processes hook results, returning the blocking command and reason if
    /// any hook blocked.
    fn process_results(results: &[(String, HookExecutionResult)]) -> Option<(String, String)> {
        results
            .iter()
            .find_map(|(cmd, result)| Self::check_blocking(cmd, result))
    }

    /// Collects `additionalContext` strings from all successful hook results,
    /// paired with the command that produced them.
    fn collect_additional_context(
        results: &[(String, HookExecutionResult)],
    ) -> Vec<(String, String)> {
        let mut contexts = Vec::new();
        for (command, result) in results {
            if let Some(output) = result.parse_output()
                && let Some(ctx) = &output.additional_context
                && !ctx.trim().is_empty()
            {
                contexts.push((command.clone(), ctx.clone()));
            }
        }
        contexts
    }

    /// Injects collected `additionalContext` into the conversation as a plain
    /// text user message. The format matches Claude Code's transcript format:
    /// ```text
    /// {event_name} hook additional context:
    /// [{command}]: {context}
    /// ```
    /// This avoids XML-like tags that LLMs may treat as prompt injection.
    fn inject_additional_context(
        conversation: &mut Conversation,
        event_name: &str,
        contexts: &[(String, String)],
    ) {
        if contexts.is_empty() {
            return;
        }
        if let Some(ctx) = conversation.context.as_mut() {
            let mut lines = vec![format!("{event_name} hook additional context:")];
            for (command, context) in contexts {
                lines.push(format!("[{command}]: {context}"));
            }
            let content = lines.join("\n");
            ctx.messages
                .push(ContextMessage::user(content, None).into());
            debug!(
                event_name = event_name,
                context_count = contexts.len(),
                "Injected additional context from hook into conversation"
            );
        }
    }

    /// Processes PreToolUse results, extracting updated input if present.
    fn process_pre_tool_use_output(
        results: &[(String, HookExecutionResult)],
    ) -> PreToolUseDecision {
        for (_command, result) in results {
            // Exit code 2 = blocking error
            if result.is_blocking_exit() {
                let message = result
                    .blocking_message()
                    .unwrap_or("Hook blocked tool execution")
                    .to_string();
                return PreToolUseDecision::Block(message);
            }

            // Exit code 0 = check stdout for JSON decisions
            if let Some(output) = result.parse_output() {
                // Check permission decision
                if output.permission_decision.as_deref() == Some("deny") {
                    let reason = output.blocking_reason("Tool execution denied by hook");
                    return PreToolUseDecision::Block(reason);
                }

                // Check generic block decision
                if output.is_blocking() {
                    let reason = output.blocking_reason("Hook blocked tool execution");
                    return PreToolUseDecision::Block(reason);
                }

                // Check for updated input
                if output.updated_input.is_some() {
                    return PreToolUseDecision::AllowWithUpdate(output);
                }
            }
        }

        PreToolUseDecision::Allow
    }
}

/// Decision result from PreToolUse hook processing.
enum PreToolUseDecision {
    /// Allow the tool call to proceed.
    Allow,
    /// Allow but with updated input from the hook output.
    AllowWithUpdate(HookOutput),
    /// Block the tool call with the given reason.
    Block(String),
}

// --- EventHandle implementations ---

#[async_trait]
impl<I: HookCommandService> EventHandle<EventData<StartPayload>> for UserHookHandler<I> {
    async fn handle(
        &self,
        event: &mut EventData<StartPayload>,
        conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        if !self.has_hooks(&UserHookEventName::SessionStart) {
            return Ok(());
        }

        let input = self.build_base_input(
            &UserHookEventName::SessionStart,
            HookEventInput::SessionStart { source: "startup".to_string() },
        );

        self.run_hooks_and_collect(
            &UserHookEventName::SessionStart,
            Some("startup"),
            &input,
            &mut event.warnings,
            conversation,
        )
        .await;

        Ok(())
    }
}

#[async_trait]
impl<I: HookCommandService> EventHandle<EventData<RequestPayload>> for UserHookHandler<I> {
    async fn handle(
        &self,
        event: &mut EventData<RequestPayload>,
        conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        // Only fire on the first request of a turn (user-submitted prompt).
        // Subsequent iterations are internal LLM retry/tool-call loops and
        // should not re-trigger UserPromptSubmit.
        if event.payload.request_count != 0 {
            return Ok(());
        }

        if !self.has_hooks(&UserHookEventName::UserPromptSubmit) {
            return Ok(());
        }

        // Extract the last user message text as the prompt sent to the hook.
        let prompt = conversation
            .context
            .as_ref()
            .and_then(|ctx| {
                ctx.messages
                    .iter()
                    .rev()
                    .find(|m| m.has_role(Role::User))
                    .and_then(|m| m.content())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();

        let input = self.build_base_input(
            &UserHookEventName::UserPromptSubmit,
            HookEventInput::UserPromptSubmit { prompt },
        );

        let results = self
            .run_hooks_and_collect(
                &UserHookEventName::UserPromptSubmit,
                None,
                &input,
                &mut event.warnings,
                conversation,
            )
            .await;

        if let Some((command, reason)) = Self::process_results(&results) {
            debug!(
                command = command.as_str(),
                reason = reason.as_str(),
                "UserPromptSubmit hook blocked with feedback"
            );
            event
                .warnings
                .push(format!("UserPromptSubmit hook blocked: {reason}"));
            // Signal the orchestrator to suppress this prompt entirely.
            return Err(anyhow::Error::from(PromptSuppressed(reason)));
        }

        Ok(())
    }
}

#[async_trait]
impl<I: HookCommandService> EventHandle<EventData<ResponsePayload>> for UserHookHandler<I> {
    async fn handle(
        &self,
        _event: &mut EventData<ResponsePayload>,
        _conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        // FIXME: No user hook events map to Response currently
        Ok(())
    }
}

#[async_trait]
impl<I: HookCommandService> EventHandle<EventData<ToolcallStartPayload>> for UserHookHandler<I> {
    async fn handle(
        &self,
        event: &mut EventData<ToolcallStartPayload>,
        conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        if !self.has_hooks(&UserHookEventName::PreToolUse) {
            return Ok(());
        }

        // Use owned String to avoid borrow conflicts when mutating event later.
        let tool_name = event.payload.tool_call.name.as_str().to_string();
        // FIXME: Add a tool name transformer to map tool names to Forge
        // equivalents (e.g. "Bash" -> "shell") so that hook matchers written
        // for other coding assistants work correctly.

        let tool_input =
            serde_json::to_value(&event.payload.tool_call.arguments).unwrap_or_default();
        let tool_use_id = event
            .payload
            .tool_call
            .call_id
            .as_ref()
            .map(|id| id.as_str().to_string());

        let input = self.build_base_input(
            &UserHookEventName::PreToolUse,
            HookEventInput::PreToolUse { tool_name: tool_name.clone(), tool_input, tool_use_id },
        );

        let results = self
            .run_hooks_and_collect(
                &UserHookEventName::PreToolUse,
                Some(tool_name.as_str()),
                &input,
                &mut event.warnings,
                conversation,
            )
            .await;

        let decision = Self::process_pre_tool_use_output(&results);

        match decision {
            PreToolUseDecision::Allow => Ok(()),
            PreToolUseDecision::AllowWithUpdate(output) => {
                if let Some(updated_input) = output.updated_input {
                    event.payload.tool_call.arguments =
                        ToolCallArguments::Parsed(Value::Object(updated_input));
                    debug!(
                        tool_name = tool_name.as_str(),
                        "PreToolUse hook updated tool input"
                    );
                }
                Ok(())
            }
            PreToolUseDecision::Block(reason) => {
                debug!(
                    tool_name = tool_name.as_str(),
                    reason = reason.as_str(),
                    "PreToolUse hook blocked tool call"
                );
                // Return an error to signal the orchestrator to skip this tool call.
                // The orchestrator converts this into an error ToolResult visible to
                // the model.
                Err(anyhow::anyhow!(
                    "Tool call '{}' blocked by PreToolUse hook: {}",
                    tool_name,
                    reason
                ))
            }
        }
    }
}

#[async_trait]
impl<I: HookCommandService> EventHandle<EventData<ToolcallEndPayload>> for UserHookHandler<I> {
    async fn handle(
        &self,
        event: &mut EventData<ToolcallEndPayload>,
        conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        let is_error = event.payload.result.is_error();
        let event_name = if is_error {
            UserHookEventName::PostToolUseFailure
        } else {
            UserHookEventName::PostToolUse
        };

        if !self.has_hooks(&event_name) {
            return Ok(());
        }

        let tool_name = event.payload.tool_call.name.as_str().to_string();

        let tool_input =
            serde_json::to_value(&event.payload.tool_call.arguments).unwrap_or_default();
        let tool_response = serde_json::to_value(&event.payload.result.output).unwrap_or_default();
        let tool_use_id = event
            .payload
            .tool_call
            .call_id
            .as_ref()
            .map(|id| id.as_str().to_string());

        let input = self.build_base_input(
            &event_name,
            HookEventInput::PostToolUse {
                tool_name: tool_name.to_string(),
                tool_input,
                tool_response,
                tool_use_id,
            },
        );

        let results = self
            .run_hooks_and_collect(
                &event_name,
                Some(&tool_name),
                &input,
                &mut event.warnings,
                conversation,
            )
            .await;

        // PostToolUse blocking: store the feedback on the event payload.
        // The orchestrator reads `hook_feedback` after `append_message` and
        // injects it into context at the correct position — after the tool
        // result, not before it. This ensures the LLM sees the feedback in
        // the right order.
        if let Some((command, reason)) = Self::process_results(&results) {
            debug!(
                tool_name = tool_name.as_str(),
                event = %event_name,
                command = command.as_str(),
                reason = reason.as_str(),
                "PostToolUse hook blocked, storing feedback for orchestrator injection"
            );
            let content = format!("{event_name}:{tool_name} hook feedback:\n[{command}]: {reason}");
            event.payload.hook_feedback = Some(content.clone());
            event
                .warnings
                .push(format!("{event_name}:{tool_name} hook blocked: {reason}"));
        }

        Ok(())
    }
}

#[async_trait]
impl<I: HookCommandService> EventHandle<EventData<EndPayload>> for UserHookHandler<I> {
    async fn handle(
        &self,
        event: &mut EventData<EndPayload>,
        conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        // Fire SessionEnd hooks
        if self.has_hooks(&UserHookEventName::SessionEnd) {
            let input =
                self.build_base_input(&UserHookEventName::SessionEnd, HookEventInput::Empty {});
            self.run_hooks_and_collect(
                &UserHookEventName::SessionEnd,
                None,
                &input,
                &mut event.warnings,
                conversation,
            )
            .await;
        }

        // Fire Stop hooks
        if !self.has_hooks(&UserHookEventName::Stop) {
            return Ok(());
        }

        let stop_hook_active = event.payload.stop_hook_active;

        // Extract the last assistant message text for the Stop hook payload.
        let last_assistant_message = conversation.context.as_ref().and_then(|ctx| {
            ctx.messages
                .iter()
                .rev()
                .find(|m| m.has_role(Role::Assistant))
                .and_then(|m| m.content())
                .map(|s| s.to_string())
        });

        let input = self.build_base_input(
            &UserHookEventName::Stop,
            HookEventInput::Stop { stop_hook_active, last_assistant_message },
        );

        let results = self
            .run_hooks_and_collect(
                &UserHookEventName::Stop,
                None,
                &input,
                &mut event.warnings,
                conversation,
            )
            .await;

        if let Some((command, reason)) = Self::process_results(&results) {
            debug!(
                command = command.as_str(),
                reason = reason.as_str(),
                stop_hook_active = stop_hook_active,
                "Stop hook blocked, injecting feedback for continuation"
            );
            // Inject the blocking reason as a conversation message. The
            // orchestrator detects that conversation.len() increased and
            // resets should_yield to false, causing another LLM turn.
            // This matches Claude Code's stop-hook continuation behavior.
            if let Some(ctx) = conversation.context.as_mut() {
                let content = format!("Stop hook feedback:\n[{command}]: {reason}");
                ctx.messages
                    .push(ContextMessage::user(content, None).into());
            }
            // Mark the next End invocation as stop_hook_active so hook
            // scripts can detect re-entrancy and avoid infinite loops.
            event.payload.stop_hook_active = true;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use forge_config::{UserHookEntry, UserHookEventName, UserHookMatcherGroup, UserHookType};
    use forge_domain::{CommandOutput, HookExecutionResult};
    use pretty_assertions::assert_eq;

    use super::*;

    /// A no-op service stub for tests that only exercise config/matching logic.
    #[derive(Clone)]
    struct NullInfra;

    #[async_trait::async_trait]
    impl HookCommandService for NullInfra {
        async fn execute_command_with_input(
            &self,
            command: String,
            _working_dir: PathBuf,
            _stdin_input: String,
            _env_vars: HashMap<String, String>,
        ) -> anyhow::Result<CommandOutput> {
            Ok(CommandOutput {
                command,
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    fn null_handler(config: UserHookConfig) -> UserHookHandler<NullInfra> {
        UserHookHandler::new(
            NullInfra,
            BTreeMap::new(),
            config,
            PathBuf::from("/tmp"),
            "sess-1".to_string(),
        )
    }

    /// Configurable stub that returns a fixed `CommandOutput` for every call.
    /// Replaces all single-purpose inline stubs (BlockExit2, JsonBlockInfra,
    /// ContinueFalseInfra, Exit1Infra, StopBlockInfra, etc.).
    #[derive(Clone)]
    struct StubInfra {
        output: forge_domain::CommandOutput,
    }

    impl StubInfra {
        fn new(exit_code: Option<i32>, stdout: &str, stderr: &str) -> Self {
            Self {
                output: forge_domain::CommandOutput {
                    command: String::new(),
                    exit_code,
                    stdout: stdout.to_string(),
                    stderr: stderr.to_string(),
                },
            }
        }
    }

    #[async_trait::async_trait]
    impl HookCommandService for StubInfra {
        async fn execute_command_with_input(
            &self,
            command: String,
            _working_dir: PathBuf,
            _stdin_input: String,
            _env_vars: HashMap<String, String>,
        ) -> anyhow::Result<forge_domain::CommandOutput> {
            let mut out = self.output.clone();
            out.command = command;
            Ok(out)
        }
    }

    fn handler_for_event<I>(infra: I, event_json: &str) -> UserHookHandler<I> {
        let config: UserHookConfig = serde_json::from_str(event_json).unwrap();
        UserHookHandler::new(
            infra,
            BTreeMap::new(),
            config,
            PathBuf::from("/tmp"),
            "sess-test".to_string(),
        )
    }

    fn make_entry(command: &str) -> UserHookEntry {
        UserHookEntry {
            hook_type: UserHookType::Command,
            command: Some(command.to_string()),
            timeout: None,
        }
    }

    fn make_group(matcher: Option<&str>, commands: &[&str]) -> UserHookMatcherGroup {
        UserHookMatcherGroup {
            matcher: matcher.map(|s| s.to_string()),
            hooks: commands.iter().map(|c| make_entry(c)).collect(),
        }
    }

    /// Builds a regex cache from a slice of matcher groups, mirroring the
    /// logic in `UserHookHandler::build_regex_cache` for test use.
    fn regex_cache_from_groups(groups: &[UserHookMatcherGroup]) -> HashMap<String, Regex> {
        let mut cache = HashMap::new();
        for group in groups {
            if let Some(pattern) = &group.matcher
                && !pattern.is_empty()
                && !cache.contains_key(pattern)
                && let Ok(re) = Regex::new(pattern)
            {
                cache.insert(pattern.clone(), re);
            }
        }
        cache
    }

    #[test]
    fn test_find_matching_hooks_no_matcher_fires_unconditionally() {
        let groups = vec![make_group(None, &["echo hi"])];
        let cache = regex_cache_from_groups(&groups);
        let actual =
            UserHookHandler::<NullInfra>::find_matching_hooks(&groups, Some("Bash"), &cache);
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].command, Some("echo hi".to_string()));
    }

    #[test]
    fn test_find_matching_hooks_no_matcher_fires_without_subject() {
        let groups = vec![make_group(None, &["echo hi"])];
        let cache = regex_cache_from_groups(&groups);
        let actual = UserHookHandler::<NullInfra>::find_matching_hooks(&groups, None, &cache);
        assert_eq!(actual.len(), 1);
    }

    #[test]
    fn test_find_matching_hooks_regex_match() {
        let groups = vec![make_group(Some("Bash"), &["block.sh"])];
        let cache = regex_cache_from_groups(&groups);
        let actual =
            UserHookHandler::<NullInfra>::find_matching_hooks(&groups, Some("Bash"), &cache);
        assert_eq!(actual.len(), 1);
    }

    #[test]
    fn test_find_matching_hooks_regex_no_match() {
        let groups = vec![make_group(Some("Bash"), &["block.sh"])];
        let cache = regex_cache_from_groups(&groups);
        let actual =
            UserHookHandler::<NullInfra>::find_matching_hooks(&groups, Some("Write"), &cache);
        assert!(actual.is_empty());
    }

    #[test]
    fn test_find_matching_hooks_regex_partial_match() {
        let groups = vec![make_group(Some("Bash|Write"), &["check.sh"])];
        let cache = regex_cache_from_groups(&groups);
        let actual =
            UserHookHandler::<NullInfra>::find_matching_hooks(&groups, Some("Bash"), &cache);
        assert_eq!(actual.len(), 1);
    }

    #[test]
    fn test_find_matching_hooks_matcher_but_no_subject() {
        let groups = vec![make_group(Some("Bash"), &["block.sh"])];
        let cache = regex_cache_from_groups(&groups);
        let actual = UserHookHandler::<NullInfra>::find_matching_hooks(&groups, None, &cache);
        assert!(actual.is_empty());
    }

    #[test]
    fn test_find_matching_hooks_empty_matcher_fires_without_subject() {
        let groups = vec![make_group(Some(""), &["stop-hook.sh"])];
        let cache = regex_cache_from_groups(&groups);
        let actual = UserHookHandler::<NullInfra>::find_matching_hooks(&groups, None, &cache);
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].command, Some("stop-hook.sh".to_string()));
    }

    #[test]
    fn test_find_matching_hooks_empty_matcher_fires_with_subject() {
        let groups = vec![make_group(Some(""), &["pre-tool.sh"])];
        let cache = regex_cache_from_groups(&groups);
        let actual =
            UserHookHandler::<NullInfra>::find_matching_hooks(&groups, Some("Bash"), &cache);
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].command, Some("pre-tool.sh".to_string()));
    }

    #[test]
    fn test_find_matching_hooks_invalid_regex_skipped() {
        let groups = vec![make_group(Some("[invalid"), &["block.sh"])];
        let cache = regex_cache_from_groups(&groups);
        let actual =
            UserHookHandler::<NullInfra>::find_matching_hooks(&groups, Some("anything"), &cache);
        assert!(actual.is_empty());
    }

    #[test]
    fn test_find_matching_hooks_multiple_groups() {
        let groups = vec![
            make_group(Some("Bash"), &["bash-hook.sh"]),
            make_group(Some("Write"), &["write-hook.sh"]),
            make_group(None, &["always.sh"]),
        ];
        let cache = regex_cache_from_groups(&groups);
        let actual =
            UserHookHandler::<NullInfra>::find_matching_hooks(&groups, Some("Bash"), &cache);
        assert_eq!(actual.len(), 2); // Bash match + unconditional
    }

    #[test]
    fn test_process_pre_tool_use_output_allow_on_success() {
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::process_pre_tool_use_output(&results);
        assert!(matches!(actual, PreToolUseDecision::Allow));
    }

    #[test]
    fn test_process_pre_tool_use_output_block_on_exit_2() {
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(2),
                stdout: String::new(),
                stderr: "Blocked: dangerous command".to_string(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::process_pre_tool_use_output(&results);
        assert!(
            matches!(actual, PreToolUseDecision::Block(msg) if msg.contains("dangerous command"))
        );
    }

    #[test]
    fn test_process_pre_tool_use_output_block_on_deny() {
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(0),
                stdout: r#"{"permissionDecision": "deny", "reason": "Not allowed"}"#.to_string(),
                stderr: String::new(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::process_pre_tool_use_output(&results);
        assert!(matches!(actual, PreToolUseDecision::Block(msg) if msg == "Not allowed"));
    }

    #[test]
    fn test_process_pre_tool_use_output_block_on_decision() {
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(0),
                stdout: r#"{"decision": "block", "reason": "Blocked by policy"}"#.to_string(),
                stderr: String::new(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::process_pre_tool_use_output(&results);
        assert!(matches!(actual, PreToolUseDecision::Block(msg) if msg == "Blocked by policy"));
    }

    #[test]
    fn test_process_pre_tool_use_output_non_blocking_error_allows() {
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(1),
                stdout: String::new(),
                stderr: "some error".to_string(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::process_pre_tool_use_output(&results);
        assert!(matches!(actual, PreToolUseDecision::Allow));
    }

    #[test]
    fn test_process_results_no_blocking() {
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::process_results(&results);
        assert!(actual.is_none());
    }

    #[test]
    fn test_process_results_blocking_exit_code() {
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(2),
                stdout: String::new(),
                stderr: "stop reason".to_string(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::process_results(&results);
        assert_eq!(
            actual,
            Some(("test-cmd".to_string(), "stop reason".to_string()))
        );
    }

    #[test]
    fn test_process_results_blocking_json_decision() {
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(0),
                stdout: r#"{"decision": "block", "reason": "keep going"}"#.to_string(),
                stderr: String::new(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::process_results(&results);
        assert_eq!(
            actual,
            Some(("test-cmd".to_string(), "keep going".to_string()))
        );
    }

    #[test]
    fn test_has_hooks_returns_false_for_empty_config() {
        let config = UserHookConfig::new();
        let handler = null_handler(config);
        assert!(!handler.has_hooks(&UserHookEventName::PreToolUse));
    }

    #[test]
    fn test_has_hooks_returns_true_when_configured() {
        let json = r#"{"PreToolUse": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#;
        let config: UserHookConfig = serde_json::from_str(json).unwrap();
        let handler = null_handler(config);
        assert!(handler.has_hooks(&UserHookEventName::PreToolUse));
        assert!(!handler.has_hooks(&UserHookEventName::Stop));
    }

    #[test]
    fn test_process_pre_tool_use_output_allow_with_update_detected() {
        // A hook that returns updatedInput should produce AllowWithUpdate with the
        // correct updated_input value.
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(0),
                stdout: r#"{"updatedInput": {"command": "echo safe"}}"#.to_string(),
                stderr: String::new(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::process_pre_tool_use_output(&results);
        let expected_map =
            serde_json::Map::from_iter([("command".to_string(), serde_json::json!("echo safe"))]);
        assert!(
            matches!(&actual, PreToolUseDecision::AllowWithUpdate(output) if output.updated_input == Some(expected_map))
        );
    }

    #[tokio::test]
    async fn test_allow_with_update_modifies_tool_call_arguments() {
        // When a PreToolUse hook returns updatedInput, the handler must
        // overwrite event.payload.tool_call.arguments with the new value.
        use forge_domain::{
            Agent, EventData, ModelId, ProviderId, ToolCallArguments, ToolCallFull,
            ToolcallStartPayload,
        };

        let json = r#"{"PreToolUse": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#;
        let config: UserHookConfig = serde_json::from_str(json).unwrap();

        let handler = UserHookHandler::new(
            StubInfra::new(Some(0), r#"{"updatedInput": {"command": "echo safe"}}"#, ""),
            BTreeMap::new(),
            config,
            PathBuf::from("/tmp"),
            "sess-test".to_string(),
        );

        let agent = Agent::new(
            "test-agent",
            ProviderId::from("test-provider".to_string()),
            ModelId::new("test-model"),
        );
        let original_args = ToolCallArguments::from_json(r#"{"command": "rm -rf /"}"#);
        let tool_call = ToolCallFull::new("shell").arguments(original_args);
        let mut event = EventData::new(
            agent,
            ModelId::new("test-model"),
            ToolcallStartPayload::new(tool_call),
        );
        let mut conversation = forge_domain::Conversation::generate();

        handler.handle(&mut event, &mut conversation).await.unwrap();

        let actual_args = event.payload.tool_call.arguments.parse().unwrap();
        let expected_args = serde_json::json!({"command": "echo safe"});
        assert_eq!(actual_args, expected_args);
    }

    #[test]
    fn test_allow_with_update_none_updated_input_leaves_args_unchanged() {
        // When HookOutput has updated_input = None (e.g. only
        // `{"permissionDecision": "allow"}`), AllowWithUpdate should not
        // overwrite the original arguments.
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(0),
                stdout: r#"{"permissionDecision": "allow"}"#.to_string(),
                stderr: String::new(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::process_pre_tool_use_output(&results);
        // permissionDecision "allow" with no updatedInput => plain Allow
        assert!(matches!(actual, PreToolUseDecision::Allow));
    }

    #[test]
    fn test_allow_with_update_empty_object() {
        // updatedInput is an empty object — still a valid update.
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(0),
                stdout: r#"{"updatedInput": {}}"#.to_string(),
                stderr: String::new(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::process_pre_tool_use_output(&results);
        let expected_map = serde_json::Map::new();
        assert!(
            matches!(&actual, PreToolUseDecision::AllowWithUpdate(output) if output.updated_input == Some(expected_map))
        );
    }

    #[test]
    fn test_allow_with_update_complex_nested_input() {
        // updatedInput with nested objects and arrays.
        let results = vec![("test-cmd".to_string(), HookExecutionResult {
            exit_code: Some(0),
            stdout: r#"{"updatedInput": {"file_path": "/safe/path", "options": {"recursive": true, "depth": 3}, "tags": ["a", "b"]}}"#.to_string(),
            stderr: String::new(),
        })];
        let actual = UserHookHandler::<NullInfra>::process_pre_tool_use_output(&results);
        let expected_map = serde_json::Map::from_iter([
            ("file_path".to_string(), serde_json::json!("/safe/path")),
            (
                "options".to_string(),
                serde_json::json!({"recursive": true, "depth": 3}),
            ),
            ("tags".to_string(), serde_json::json!(["a", "b"])),
        ]);
        assert!(
            matches!(&actual, PreToolUseDecision::AllowWithUpdate(output) if output.updated_input == Some(expected_map))
        );
    }

    #[test]
    fn test_block_takes_priority_over_updated_input() {
        // If a hook returns both decision=block AND updatedInput, the block
        // must win because blocking is checked before updatedInput.
        let results = vec![("test-cmd".to_string(), HookExecutionResult {
            exit_code: Some(0),
            stdout: r#"{"decision": "block", "reason": "nope", "updatedInput": {"command": "echo safe"}}"#.to_string(),
            stderr: String::new(),
        })];
        let actual = UserHookHandler::<NullInfra>::process_pre_tool_use_output(&results);
        assert!(matches!(actual, PreToolUseDecision::Block(msg) if msg == "nope"));
    }

    #[test]
    fn test_deny_takes_priority_over_updated_input() {
        // permissionDecision=deny should block even if updatedInput is present.
        let results = vec![("test-cmd".to_string(), HookExecutionResult {
            exit_code: Some(0),
            stdout: r#"{"permissionDecision": "deny", "reason": "forbidden", "updatedInput": {"command": "echo safe"}}"#.to_string(),
            stderr: String::new(),
        })];
        let actual = UserHookHandler::<NullInfra>::process_pre_tool_use_output(&results);
        assert!(matches!(actual, PreToolUseDecision::Block(msg) if msg == "forbidden"));
    }

    #[test]
    fn test_exit_code_2_blocks_even_with_updated_input_in_stdout() {
        // Exit code 2 is a hard block regardless of stdout content.
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(2),
                stdout: r#"{"updatedInput": {"command": "echo safe"}}"#.to_string(),
                stderr: "hard block".to_string(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::process_pre_tool_use_output(&results);
        assert!(matches!(actual, PreToolUseDecision::Block(msg) if msg.contains("hard block")));
    }

    #[test]
    fn test_multiple_results_first_update_wins() {
        // When multiple hooks run and the first returns updatedInput, that
        // result is used (iteration stops at first non-Allow decision).
        let results = vec![
            (
                "test-cmd-1".to_string(),
                HookExecutionResult {
                    exit_code: Some(0),
                    stdout: r#"{"updatedInput": {"command": "first"}}"#.to_string(),
                    stderr: String::new(),
                },
            ),
            (
                "test-cmd-2".to_string(),
                HookExecutionResult {
                    exit_code: Some(0),
                    stdout: r#"{"updatedInput": {"command": "second"}}"#.to_string(),
                    stderr: String::new(),
                },
            ),
        ];
        let actual = UserHookHandler::<NullInfra>::process_pre_tool_use_output(&results);
        let expected_map =
            serde_json::Map::from_iter([("command".to_string(), serde_json::json!("first"))]);
        assert!(
            matches!(&actual, PreToolUseDecision::AllowWithUpdate(output) if output.updated_input == Some(expected_map))
        );
    }

    #[test]
    fn test_multiple_results_block_before_update() {
        // A block from an earlier hook prevents a later hook's updatedInput
        // from being applied.
        let results = vec![
            (
                "test-cmd-1".to_string(),
                HookExecutionResult {
                    exit_code: Some(2),
                    stdout: String::new(),
                    stderr: "blocked first".to_string(),
                },
            ),
            (
                "test-cmd-2".to_string(),
                HookExecutionResult {
                    exit_code: Some(0),
                    stdout: r#"{"updatedInput": {"command": "safe"}}"#.to_string(),
                    stderr: String::new(),
                },
            ),
        ];
        let actual = UserHookHandler::<NullInfra>::process_pre_tool_use_output(&results);
        assert!(matches!(actual, PreToolUseDecision::Block(msg) if msg.contains("blocked first")));
    }

    #[test]
    fn test_non_blocking_error_then_update() {
        // A non-blocking error (exit 1) from the first hook is logged but
        // doesn't prevent a subsequent hook from returning updatedInput.
        let results = vec![
            (
                "test-cmd-1".to_string(),
                HookExecutionResult {
                    exit_code: Some(1),
                    stdout: String::new(),
                    stderr: "warning".to_string(),
                },
            ),
            (
                "test-cmd-2".to_string(),
                HookExecutionResult {
                    exit_code: Some(0),
                    stdout: r#"{"updatedInput": {"command": "safe"}}"#.to_string(),
                    stderr: String::new(),
                },
            ),
        ];
        let actual = UserHookHandler::<NullInfra>::process_pre_tool_use_output(&results);
        let expected_map =
            serde_json::Map::from_iter([("command".to_string(), serde_json::json!("safe"))]);
        assert!(
            matches!(&actual, PreToolUseDecision::AllowWithUpdate(output) if output.updated_input == Some(expected_map))
        );
    }

    #[tokio::test]
    async fn test_allow_with_update_no_updated_input_preserves_original() {
        // When the hook returns exit 0 with empty stdout (no updatedInput),
        // the original tool call arguments must remain untouched.
        use forge_domain::{
            Agent, EventData, ModelId, ProviderId, ToolCallArguments, ToolCallFull,
            ToolcallStartPayload,
        };

        let json = r#"{"PreToolUse": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#;
        let config: UserHookConfig = serde_json::from_str(json).unwrap();
        // NullInfra returns exit 0 + empty stdout => Allow
        let handler = null_handler(config);

        let agent = Agent::new(
            "test-agent",
            ProviderId::from("test-provider".to_string()),
            ModelId::new("test-model"),
        );
        let original_args = ToolCallArguments::from_json(r#"{"command": "ls"}"#);
        let tool_call = ToolCallFull::new("shell").arguments(original_args);
        let mut event = EventData::new(
            agent,
            ModelId::new("test-model"),
            ToolcallStartPayload::new(tool_call),
        );
        let mut conversation = forge_domain::Conversation::generate();

        handler.handle(&mut event, &mut conversation).await.unwrap();

        // Arguments must still be the original value
        let actual_args = event.payload.tool_call.arguments.parse().unwrap();
        let expected_args = serde_json::json!({"command": "ls"});
        assert_eq!(actual_args, expected_args);
    }

    #[tokio::test]
    async fn test_allow_with_update_replaces_unparsed_with_parsed() {
        // Original arguments are Unparsed (raw string from LLM). After
        // AllowWithUpdate, they should become Parsed(Value).
        use forge_domain::{
            Agent, EventData, ModelId, ProviderId, ToolCallArguments, ToolCallFull,
            ToolcallStartPayload,
        };

        let json = r#"{"PreToolUse": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#;
        let config: UserHookConfig = serde_json::from_str(json).unwrap();

        let handler = UserHookHandler::new(
            StubInfra::new(
                Some(0),
                r#"{"updatedInput": {"file_path": "/safe/file.txt", "content": "hello"}}"#,
                "",
            ),
            BTreeMap::new(),
            config,
            PathBuf::from("/tmp"),
            "sess-test2".to_string(),
        );

        let agent = Agent::new(
            "test-agent",
            ProviderId::from("test-provider".to_string()),
            ModelId::new("test-model"),
        );
        // Start with Unparsed arguments
        let original_args =
            ToolCallArguments::from_json(r#"{"file_path": "/etc/passwd", "content": "evil"}"#);
        assert!(matches!(original_args, ToolCallArguments::Unparsed(_)));

        let tool_call = ToolCallFull::new("write").arguments(original_args);
        let mut event = EventData::new(
            agent,
            ModelId::new("test-model"),
            ToolcallStartPayload::new(tool_call),
        );
        let mut conversation = forge_domain::Conversation::generate();

        handler.handle(&mut event, &mut conversation).await.unwrap();

        // After update, arguments should be Parsed
        assert!(matches!(
            event.payload.tool_call.arguments,
            ToolCallArguments::Parsed(_)
        ));
        let actual_args = event.payload.tool_call.arguments.parse().unwrap();
        let expected_args = serde_json::json!({"file_path": "/safe/file.txt", "content": "hello"});
        assert_eq!(actual_args, expected_args);
    }

    #[tokio::test]
    async fn test_block_returns_error_and_preserves_original_args() {
        // When a hook blocks, handle() returns Err and the event arguments
        // remain unchanged.
        use forge_domain::{
            Agent, EventData, ModelId, ProviderId, ToolCallArguments, ToolCallFull,
            ToolcallStartPayload,
        };

        let json = r#"{"PreToolUse": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#;
        let config: UserHookConfig = serde_json::from_str(json).unwrap();

        let handler = UserHookHandler::new(
            StubInfra::new(Some(2), "", "dangerous operation"),
            BTreeMap::new(),
            config,
            PathBuf::from("/tmp"),
            "sess-block".to_string(),
        );

        let agent = Agent::new(
            "test-agent",
            ProviderId::from("test-provider".to_string()),
            ModelId::new("test-model"),
        );
        let original_args = ToolCallArguments::from_json(r#"{"command": "rm -rf /"}"#);
        let tool_call = ToolCallFull::new("shell").arguments(original_args);
        let mut event = EventData::new(
            agent,
            ModelId::new("test-model"),
            ToolcallStartPayload::new(tool_call),
        );
        let mut conversation = forge_domain::Conversation::generate();

        let result = handler.handle(&mut event, &mut conversation).await;

        // Should be an error
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("blocked by PreToolUse hook"));
        assert!(err_msg.contains("dangerous operation"));

        // Arguments must still be the original value (not modified)
        let actual_args = event.payload.tool_call.arguments.parse().unwrap();
        let expected_args = serde_json::json!({"command": "rm -rf /"});
        assert_eq!(actual_args, expected_args);
    }

    #[test]
    fn test_process_results_blocking_continue_false() {
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(0),
                stdout: r#"{"continue": false, "stopReason": "task complete"}"#.to_string(),
                stderr: String::new(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::process_results(&results);
        assert_eq!(
            actual,
            Some(("test-cmd".to_string(), "task complete".to_string()))
        );
    }

    #[test]
    fn test_process_pre_tool_use_output_block_on_continue_false() {
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(0),
                stdout: r#"{"continue": false, "stopReason": "no more tools"}"#.to_string(),
                stderr: String::new(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::process_pre_tool_use_output(&results);
        assert!(matches!(actual, PreToolUseDecision::Block(msg) if msg == "no more tools"));
    }

    #[test]
    fn test_process_results_stop_reason_fallback() {
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(0),
                stdout: r#"{"decision": "block", "stopReason": "fallback reason"}"#.to_string(),
                stderr: String::new(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::process_results(&results);
        assert_eq!(
            actual,
            Some(("test-cmd".to_string(), "fallback reason".to_string()))
        );
    }

    #[test]
    fn test_process_results_reason_over_stop_reason() {
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(0),
                stdout: r#"{"decision": "block", "reason": "primary", "stopReason": "secondary"}"#
                    .to_string(),
                stderr: String::new(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::process_results(&results);
        assert_eq!(
            actual,
            Some(("test-cmd".to_string(), "primary".to_string()))
        );
    }

    // =========================================================================
    // Tests: UserPromptSubmit blocking must return Err(PromptSuppressed)
    // =========================================================================

    /// Helper: creates a RequestPayload EventData with the given request_count.
    fn request_event(request_count: usize) -> EventData<forge_domain::RequestPayload> {
        use forge_domain::{Agent, ModelId, ProviderId};
        let agent = Agent::new(
            "test-agent",
            ProviderId::from("test-provider".to_string()),
            ModelId::new("test-model"),
        );
        EventData::new(
            agent,
            ModelId::new("test-model"),
            forge_domain::RequestPayload::new(request_count),
        )
    }

    /// Helper: creates a Conversation with a context containing one user
    /// message.
    fn conversation_with_user_msg(msg: &str) -> forge_domain::Conversation {
        let mut conv = forge_domain::Conversation::generate();
        let mut ctx = forge_domain::Context::default();
        ctx.messages
            .push(forge_domain::ContextMessage::user(msg.to_string(), None).into());
        conv.context = Some(ctx);
        conv
    }

    #[tokio::test]
    async fn test_user_prompt_submit_block_exit2_returns_error() {
        // TC16: exit code 2 must return PromptSuppressed error.
        let handler = handler_for_event(
            StubInfra::new(Some(2), "", "policy violation"),
            r#"{"UserPromptSubmit": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = request_event(0);
        let mut conversation = conversation_with_user_msg("hello");

        let result = handler.handle(&mut event, &mut conversation).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.downcast_ref::<forge_domain::PromptSuppressed>()
                .is_some()
        );
        assert!(err.to_string().contains("policy violation"));

        // Warning should have been pushed to event.warnings
        assert_eq!(event.warnings.len(), 1);
        assert!(event.warnings[0].contains("policy violation"));
    }

    #[tokio::test]
    async fn test_user_prompt_submit_block_json_decision_returns_error() {
        // JSON {"decision":"block","reason":"Content policy"} must block.
        let handler = handler_for_event(
            StubInfra::new(
                Some(0),
                r#"{"decision":"block","reason":"Content policy"}"#,
                "",
            ),
            r#"{"UserPromptSubmit": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = request_event(0);
        let mut conversation = conversation_with_user_msg("test");

        let result = handler.handle(&mut event, &mut conversation).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.downcast_ref::<forge_domain::PromptSuppressed>()
                .is_some()
        );
        assert!(err.to_string().contains("Content policy"));
    }

    #[tokio::test]
    async fn test_user_prompt_submit_block_continue_false_returns_error() {
        // {"continue":false,"reason":"Blocked by admin"} must block.
        let handler = handler_for_event(
            StubInfra::new(
                Some(0),
                r#"{"continue":false,"reason":"Blocked by admin"}"#,
                "",
            ),
            r#"{"UserPromptSubmit": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = request_event(0);
        let mut conversation = conversation_with_user_msg("test");

        let result = handler.handle(&mut event, &mut conversation).await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .downcast_ref::<forge_domain::PromptSuppressed>()
                .is_some()
        );
    }

    #[tokio::test]
    async fn test_user_prompt_submit_allow_returns_ok() {
        // Exit 0 + empty stdout => allow, no feedback injected.
        let handler = handler_for_event(
            NullInfra,
            r#"{"UserPromptSubmit": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = request_event(0);
        let mut conversation = conversation_with_user_msg("hello");
        let original_msg_count = conversation.context.as_ref().unwrap().messages.len();

        let result = handler.handle(&mut event, &mut conversation).await;

        assert!(result.is_ok());
        let actual_msg_count = conversation.context.as_ref().unwrap().messages.len();
        assert_eq!(actual_msg_count, original_msg_count);
    }

    #[tokio::test]
    async fn test_user_prompt_submit_non_blocking_error_returns_ok() {
        // Exit code 1 is a non-blocking error — must NOT block.
        let handler = handler_for_event(
            StubInfra::new(Some(1), "", "some error"),
            r#"{"UserPromptSubmit": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = request_event(0);
        let mut conversation = conversation_with_user_msg("hello");

        let result = handler.handle(&mut event, &mut conversation).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_user_prompt_submit_skipped_on_subsequent_requests() {
        // request_count > 0 means it's a retry, not a user prompt.
        let handler = handler_for_event(
            NullInfra,
            r#"{"UserPromptSubmit": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = request_event(1); // subsequent request
        let mut conversation = conversation_with_user_msg("hello");
        let original_msg_count = conversation.context.as_ref().unwrap().messages.len();

        let result = handler.handle(&mut event, &mut conversation).await;

        assert!(result.is_ok());
        let actual_msg_count = conversation.context.as_ref().unwrap().messages.len();
        assert_eq!(actual_msg_count, original_msg_count);
    }

    // =========================================================================
    // Stop hook tests: Stop hooks fire and inject feedback for continuation
    // =========================================================================

    /// Helper: creates an EndPayload EventData with optional stop_hook_active.
    fn end_event() -> EventData<forge_domain::EndPayload> {
        use forge_domain::{Agent, ModelId, ProviderId};
        let agent = Agent::new(
            "test-agent",
            ProviderId::from("test-provider".to_string()),
            ModelId::new("test-model"),
        );
        EventData::new(
            agent,
            ModelId::new("test-model"),
            forge_domain::EndPayload { stop_hook_active: false },
        )
    }

    #[tokio::test]
    async fn test_stop_hook_exit_code_2_injects_message_and_sets_active() {
        let handler = handler_for_event(
            StubInfra::new(Some(2), "", "keep working"),
            r#"{"Stop": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = end_event();
        let mut conversation = conversation_with_user_msg("hello");
        let original_msg_count = conversation.context.as_ref().unwrap().messages.len();

        let result = handler.handle(&mut event, &mut conversation).await;

        // Result is Ok (never errors)
        assert!(result.is_ok());
        // A conversation message should have been injected for continuation
        let actual_msg_count = conversation.context.as_ref().unwrap().messages.len();
        assert_eq!(actual_msg_count, original_msg_count + 1);
        // The injected message should contain the blocking reason
        let last_msg = conversation
            .context
            .as_ref()
            .unwrap()
            .messages
            .last()
            .unwrap();
        let content = last_msg.content().unwrap();
        assert!(content.contains("keep working"));
        assert!(content.contains("Stop hook feedback"));
        // stop_hook_active should be set to true for the next iteration
        assert!(event.payload.stop_hook_active);
    }

    #[tokio::test]
    async fn test_stop_hook_allow_returns_ok() {
        let handler = handler_for_event(
            NullInfra,
            r#"{"Stop": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = end_event();
        let mut conversation = conversation_with_user_msg("hello");
        let original_msg_count = conversation.context.as_ref().unwrap().messages.len();

        let result = handler.handle(&mut event, &mut conversation).await;

        assert!(result.is_ok());
        // No continue message should be injected
        let actual_msg_count = conversation.context.as_ref().unwrap().messages.len();
        assert_eq!(actual_msg_count, original_msg_count);
    }

    #[tokio::test]
    async fn test_stop_hook_json_continue_false_injects_message() {
        let handler = handler_for_event(
            StubInfra::new(
                Some(0),
                r#"{"continue":false,"stopReason":"keep working"}"#,
                "",
            ),
            r#"{"Stop": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = end_event();
        let mut conversation = conversation_with_user_msg("hello");
        let original_msg_count = conversation.context.as_ref().unwrap().messages.len();

        let result = handler.handle(&mut event, &mut conversation).await;

        // Result is Ok (never errors)
        assert!(result.is_ok());
        // A conversation message should have been injected
        let actual_msg_count = conversation.context.as_ref().unwrap().messages.len();
        assert_eq!(actual_msg_count, original_msg_count + 1);
        // stop_hook_active should be set to true
        assert!(event.payload.stop_hook_active);
    }

    #[tokio::test]
    async fn test_session_end_and_stop_hooks_both_fire() {
        // Both SessionEnd and Stop hooks should execute. Stop hooks inject
        // messages for continuation when they block.
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

        #[derive(Clone)]
        struct CountingInfra {
            call_count: Arc<AtomicU32>,
        }

        #[async_trait::async_trait]
        impl HookCommandService for CountingInfra {
            async fn execute_command_with_input(
                &self,
                command: String,
                _: PathBuf,
                _: String,
                _: HashMap<String, String>,
            ) -> anyhow::Result<forge_domain::CommandOutput> {
                self.call_count.fetch_add(1, AtomicOrdering::SeqCst);
                // Return exit 2 (blocking)
                Ok(forge_domain::CommandOutput {
                    command,
                    exit_code: Some(2),
                    stdout: String::new(),
                    stderr: "blocked".to_string(),
                })
            }
        }

        // Config with both SessionEnd and Stop hooks
        let json = r#"{
            "SessionEnd": [{"hooks": [{"type": "command", "command": "echo session-end"}]}],
            "Stop": [{"hooks": [{"type": "command", "command": "echo stop"}]}]
        }"#;
        let config: UserHookConfig = serde_json::from_str(json).unwrap();
        let call_count = Arc::new(AtomicU32::new(0));
        let handler = UserHookHandler::new(
            CountingInfra { call_count: call_count.clone() },
            BTreeMap::new(),
            config,
            PathBuf::from("/tmp"),
            "sess-test".to_string(),
        );

        let mut event = end_event();
        let mut conversation = conversation_with_user_msg("hello");

        let result = handler.handle(&mut event, &mut conversation).await;

        // Result is Ok
        assert!(result.is_ok());
        // Both SessionEnd AND Stop hooks should have been called (2 total)
        let actual = call_count.load(AtomicOrdering::SeqCst);
        assert_eq!(actual, 2);
        // Stop hook blocked, so stop_hook_active should be true
        assert!(event.payload.stop_hook_active);
    }

    #[tokio::test]
    async fn test_stop_hook_active_true_passed_to_hook_input() {
        // When stop_hook_active is true (re-entrant call), the hook should
        // receive it in its JSON input.
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct CapturingInfra {
            captured_input: Arc<Mutex<Option<String>>>,
        }

        #[async_trait::async_trait]
        impl HookCommandService for CapturingInfra {
            async fn execute_command_with_input(
                &self,
                command: String,
                _: PathBuf,
                input: String,
                _: HashMap<String, String>,
            ) -> anyhow::Result<forge_domain::CommandOutput> {
                *self.captured_input.lock().unwrap() = Some(input);
                Ok(forge_domain::CommandOutput {
                    command,
                    exit_code: Some(0),
                    stdout: String::new(),
                    stderr: String::new(),
                })
            }
        }

        let captured = Arc::new(Mutex::new(None));
        let handler = handler_for_event(
            CapturingInfra { captured_input: captured.clone() },
            r#"{"Stop": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        // Create event with stop_hook_active = true (simulating re-entrant call)
        let mut event = {
            use forge_domain::{Agent, ModelId, ProviderId};
            let agent = Agent::new(
                "test-agent",
                ProviderId::from("test-provider".to_string()),
                ModelId::new("test-model"),
            );
            EventData::new(
                agent,
                ModelId::new("test-model"),
                forge_domain::EndPayload { stop_hook_active: true },
            )
        };
        let mut conversation = conversation_with_user_msg("hello");

        let result = handler.handle(&mut event, &mut conversation).await;
        assert!(result.is_ok());

        // Verify the hook received stop_hook_active = true in its JSON input
        let input_json = captured.lock().unwrap().clone().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&input_json).unwrap();
        assert_eq!(parsed["stop_hook_active"], serde_json::Value::Bool(true));
    }

    #[tokio::test]
    async fn test_stop_hook_allow_does_not_inject_message() {
        // When a Stop hook allows the stop (exit 0, no blocking JSON), no
        // message should be injected and stop_hook_active should remain false.
        let handler = handler_for_event(
            NullInfra,
            r#"{"Stop": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = end_event();
        let mut conversation = conversation_with_user_msg("hello");
        let original_msg_count = conversation.context.as_ref().unwrap().messages.len();

        let result = handler.handle(&mut event, &mut conversation).await;

        assert!(result.is_ok());
        // No message injected
        let actual_msg_count = conversation.context.as_ref().unwrap().messages.len();
        assert_eq!(actual_msg_count, original_msg_count);
        // stop_hook_active should remain false
        assert!(!event.payload.stop_hook_active);
    }

    #[tokio::test]
    async fn test_stop_hook_active_false_on_initial_call() {
        // On the first call, stop_hook_active should be false in the JSON input.
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct CapturingInfra2 {
            captured_input: Arc<Mutex<Option<String>>>,
        }

        #[async_trait::async_trait]
        impl HookCommandService for CapturingInfra2 {
            async fn execute_command_with_input(
                &self,
                command: String,
                _: PathBuf,
                input: String,
                _: HashMap<String, String>,
            ) -> anyhow::Result<forge_domain::CommandOutput> {
                *self.captured_input.lock().unwrap() = Some(input);
                Ok(forge_domain::CommandOutput {
                    command,
                    exit_code: Some(0),
                    stdout: String::new(),
                    stderr: String::new(),
                })
            }
        }

        let captured = Arc::new(Mutex::new(None));
        let handler = handler_for_event(
            CapturingInfra2 { captured_input: captured.clone() },
            r#"{"Stop": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = end_event(); // stop_hook_active defaults to false
        let mut conversation = conversation_with_user_msg("hello");

        let result = handler.handle(&mut event, &mut conversation).await;
        assert!(result.is_ok());

        // Verify stop_hook_active is false in the JSON
        let input_json = captured.lock().unwrap().clone().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&input_json).unwrap();
        assert_eq!(parsed["stop_hook_active"], serde_json::Value::Bool(false));
    }

    // =========================================================================
    // BUG-3 Tests: PostToolUse feedback must use <important> wrapper
    // =========================================================================

    /// Helper: creates a ToolcallEndPayload EventData with a successful tool
    /// result.
    fn toolcall_end_event(
        tool_name: &str,
        is_error: bool,
    ) -> EventData<forge_domain::ToolcallEndPayload> {
        use forge_domain::{Agent, ModelId, ProviderId, ToolCallFull, ToolResult};
        let agent = Agent::new(
            "test-agent",
            ProviderId::from("test-provider".to_string()),
            ModelId::new("test-model"),
        );
        let tool_call = ToolCallFull::new(tool_name);
        let result = if is_error {
            ToolResult::new(tool_name).failure(anyhow::anyhow!("tool failed"))
        } else {
            ToolResult::new(tool_name).success("output data")
        };
        EventData::new(
            agent,
            ModelId::new("test-model"),
            forge_domain::ToolcallEndPayload::new(tool_call, result),
        )
    }

    #[tokio::test]
    async fn test_post_tool_use_block_injects_important_feedback() {
        let handler = handler_for_event(
            StubInfra::new(Some(2), "", "sensitive data detected"),
            r#"{"PostToolUse": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = toolcall_end_event("shell", false);
        let mut conversation = conversation_with_user_msg("hello");

        let result = handler.handle(&mut event, &mut conversation).await;

        // PostToolUse always returns Ok
        assert!(result.is_ok());

        // Warning pushed to event.warnings
        assert_eq!(event.warnings.len(), 1);
        assert!(event.warnings[0].contains("sensitive data detected"));

        // Feedback stored on payload for the orchestrator to inject after
        // append_message
        let feedback = event.payload.hook_feedback.as_ref().unwrap();
        assert!(feedback.contains("hook feedback"));
        assert!(feedback.contains("sensitive data detected"));
    }

    #[tokio::test]
    async fn test_post_tool_use_block_json_injects_feedback() {
        let handler = handler_for_event(
            StubInfra::new(
                Some(0),
                r#"{"decision":"block","reason":"PII detected"}"#,
                "",
            ),
            r#"{"PostToolUse": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = toolcall_end_event("shell", false);
        let mut conversation = conversation_with_user_msg("hello");

        let result = handler.handle(&mut event, &mut conversation).await;

        assert!(result.is_ok());
        assert_eq!(event.warnings.len(), 1);
        assert!(event.warnings[0].contains("PII detected"));

        // Feedback stored on payload for the orchestrator to inject after
        // append_message
        let feedback = event.payload.hook_feedback.as_ref().unwrap();
        assert!(feedback.contains("hook feedback"));
        assert!(feedback.contains("PII detected"));
    }

    #[tokio::test]
    async fn test_post_tool_use_allow_no_feedback() {
        let handler = handler_for_event(
            NullInfra,
            r#"{"PostToolUse": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = toolcall_end_event("shell", false);
        let mut conversation = conversation_with_user_msg("hello");
        let original_msg_count = conversation.context.as_ref().unwrap().messages.len();

        let result = handler.handle(&mut event, &mut conversation).await;

        assert!(result.is_ok());
        let actual_msg_count = conversation.context.as_ref().unwrap().messages.len();
        assert_eq!(actual_msg_count, original_msg_count);
    }

    #[tokio::test]
    async fn test_post_tool_use_non_blocking_error_no_feedback() {
        let handler = handler_for_event(
            StubInfra::new(Some(1), "", "non-blocking error"),
            r#"{"PostToolUse": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = toolcall_end_event("shell", false);
        let mut conversation = conversation_with_user_msg("hello");
        let original_msg_count = conversation.context.as_ref().unwrap().messages.len();

        let result = handler.handle(&mut event, &mut conversation).await;

        assert!(result.is_ok());
        let actual_msg_count = conversation.context.as_ref().unwrap().messages.len();
        assert_eq!(actual_msg_count, original_msg_count);
    }

    #[tokio::test]
    async fn test_post_tool_use_failure_event_fires_separately() {
        // PostToolUseFailure is a separate event from PostToolUse.
        // Configure only PostToolUseFailure hooks and fire with is_error=true.
        let handler = handler_for_event(
            StubInfra::new(Some(2), "", "error flagged"),
            r#"{"PostToolUseFailure": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );

        let mut event = toolcall_end_event("shell", true);
        let mut conversation = conversation_with_user_msg("hello");

        let result = handler.handle(&mut event, &mut conversation).await;

        assert!(result.is_ok());
        assert_eq!(event.warnings.len(), 1);
        assert!(event.warnings[0].contains("error flagged"));
    }

    #[tokio::test]
    async fn test_post_tool_use_feedback_contains_tool_name() {
        let handler = handler_for_event(
            StubInfra::new(Some(2), "", "flagged"),
            r#"{"PostToolUse": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = toolcall_end_event("shell", false);
        let mut conversation = conversation_with_user_msg("hello");

        handler.handle(&mut event, &mut conversation).await.unwrap();

        // The warning should reference the tool name
        assert_eq!(event.warnings.len(), 1);
        assert!(event.warnings[0].contains("shell"));
    }

    // =========================================================================
    // Tests: additionalContext injection
    // =========================================================================

    #[tokio::test]
    async fn test_session_start_injects_additional_context() {
        let json = r#"{"SessionStart": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#;
        let config: UserHookConfig = serde_json::from_str(json).unwrap();
        let handler = UserHookHandler::new(
            StubInfra::new(
                Some(0),
                r#"{"additionalContext": "Remember to follow coding standards"}"#,
                "",
            ),
            BTreeMap::new(),
            config,
            PathBuf::from("/tmp"),
            "sess-ctx".to_string(),
        );

        let agent = forge_domain::Agent::new(
            "test-agent",
            forge_domain::ProviderId::from("test-provider".to_string()),
            forge_domain::ModelId::new("test-model"),
        );
        let mut event = EventData::new(
            agent,
            forge_domain::ModelId::new("test-model"),
            forge_domain::StartPayload,
        );
        let mut conversation = conversation_with_user_msg("hello");
        let original_count = conversation.context.as_ref().unwrap().messages.len();

        handler.handle(&mut event, &mut conversation).await.unwrap();

        let actual_count = conversation.context.as_ref().unwrap().messages.len();
        assert_eq!(actual_count, original_count + 1);

        let last_msg = conversation
            .context
            .as_ref()
            .unwrap()
            .messages
            .last()
            .unwrap();
        let content = last_msg.content().unwrap();
        assert!(content.contains("SessionStart hook additional context"));
        assert!(content.contains("Remember to follow coding standards"));
    }

    #[tokio::test]
    async fn test_user_prompt_submit_injects_additional_context() {
        let handler = UserHookHandler::new(
            StubInfra::new(
                Some(0),
                r#"{"additionalContext": "Remember to follow coding standards"}"#,
                "",
            ),
            BTreeMap::new(),
            serde_json::from_str(
                r#"{"UserPromptSubmit": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
            )
            .unwrap(),
            PathBuf::from("/tmp"),
            "sess-ctx".to_string(),
        );

        let mut event = request_event(0);
        let mut conversation = conversation_with_user_msg("test prompt");
        let original_count = conversation.context.as_ref().unwrap().messages.len();

        handler.handle(&mut event, &mut conversation).await.unwrap();

        let actual_count = conversation.context.as_ref().unwrap().messages.len();
        assert_eq!(actual_count, original_count + 1);

        let last_msg = conversation
            .context
            .as_ref()
            .unwrap()
            .messages
            .last()
            .unwrap();
        let content = last_msg.content().unwrap();
        assert!(content.contains("UserPromptSubmit hook additional context"));
        assert!(content.contains("Remember to follow coding standards"));
    }

    #[tokio::test]
    async fn test_pre_tool_use_injects_additional_context() {
        let json = r#"{"PreToolUse": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#;
        let config: UserHookConfig = serde_json::from_str(json).unwrap();
        let handler = UserHookHandler::new(
            StubInfra::new(
                Some(0),
                r#"{"additionalContext": "Remember to follow coding standards"}"#,
                "",
            ),
            BTreeMap::new(),
            config,
            PathBuf::from("/tmp"),
            "sess-ctx".to_string(),
        );

        let agent = forge_domain::Agent::new(
            "test-agent",
            forge_domain::ProviderId::from("test-provider".to_string()),
            forge_domain::ModelId::new("test-model"),
        );
        let tool_call = forge_domain::ToolCallFull::new("shell").arguments(
            forge_domain::ToolCallArguments::from_json(r#"{"command": "ls"}"#),
        );
        let mut event = EventData::new(
            agent,
            forge_domain::ModelId::new("test-model"),
            forge_domain::ToolcallStartPayload::new(tool_call),
        );
        let mut conversation = conversation_with_user_msg("hello");
        let original_count = conversation.context.as_ref().unwrap().messages.len();

        handler.handle(&mut event, &mut conversation).await.unwrap();

        let actual_count = conversation.context.as_ref().unwrap().messages.len();
        assert_eq!(actual_count, original_count + 1);

        let last_msg = conversation
            .context
            .as_ref()
            .unwrap()
            .messages
            .last()
            .unwrap();
        let content = last_msg.content().unwrap();
        assert!(content.contains("PreToolUse hook additional context"));
        assert!(content.contains("Remember to follow coding standards"));
    }

    #[tokio::test]
    async fn test_post_tool_use_injects_additional_context() {
        let handler = handler_for_event(
            StubInfra::new(
                Some(0),
                r#"{"additionalContext": "Remember to follow coding standards"}"#,
                "",
            ),
            r#"{"PostToolUse": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = toolcall_end_event("shell", false);
        let mut conversation = conversation_with_user_msg("hello");
        let original_count = conversation.context.as_ref().unwrap().messages.len();

        handler.handle(&mut event, &mut conversation).await.unwrap();

        let actual_count = conversation.context.as_ref().unwrap().messages.len();
        assert_eq!(actual_count, original_count + 1);

        let last_msg = conversation
            .context
            .as_ref()
            .unwrap()
            .messages
            .last()
            .unwrap();
        let content = last_msg.content().unwrap();
        assert!(content.contains("PostToolUse hook additional context"));
        assert!(content.contains("Remember to follow coding standards"));
    }

    #[tokio::test]
    async fn test_no_additional_context_when_empty() {
        // NullInfra returns empty stdout => no additionalContext
        let handler = handler_for_event(
            NullInfra,
            r#"{"PostToolUse": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}"#,
        );
        let mut event = toolcall_end_event("shell", false);
        let mut conversation = conversation_with_user_msg("hello");
        let original_count = conversation.context.as_ref().unwrap().messages.len();

        handler.handle(&mut event, &mut conversation).await.unwrap();

        let actual_count = conversation.context.as_ref().unwrap().messages.len();
        assert_eq!(actual_count, original_count);
    }

    #[test]
    fn test_collect_additional_context_from_results() {
        let results = vec![
            (
                "test-cmd".to_string(),
                HookExecutionResult {
                    exit_code: Some(0),
                    stdout: r#"{"additionalContext": "first context"}"#.to_string(),
                    stderr: String::new(),
                },
            ),
            (
                "test-cmd".to_string(),
                HookExecutionResult {
                    exit_code: Some(0),
                    stdout: r#"{"additionalContext": "second context"}"#.to_string(),
                    stderr: String::new(),
                },
            ),
        ];
        let actual = UserHookHandler::<NullInfra>::collect_additional_context(&results);
        assert_eq!(
            actual,
            vec![
                ("test-cmd".to_string(), "first context".to_string()),
                ("test-cmd".to_string(), "second context".to_string())
            ]
        );
    }

    #[test]
    fn test_collect_additional_context_skips_empty() {
        let results = vec![
            (
                "test-cmd".to_string(),
                HookExecutionResult {
                    exit_code: Some(0),
                    stdout: r#"{"additionalContext": ""}"#.to_string(),
                    stderr: String::new(),
                },
            ),
            (
                "test-cmd".to_string(),
                HookExecutionResult {
                    exit_code: Some(0),
                    stdout: r#"{"additionalContext": "  "}"#.to_string(),
                    stderr: String::new(),
                },
            ),
        ];
        let actual = UserHookHandler::<NullInfra>::collect_additional_context(&results);
        assert!(actual.is_empty());
    }

    #[test]
    fn test_collect_additional_context_skips_non_success() {
        let results = vec![(
            "test-cmd".to_string(),
            HookExecutionResult {
                exit_code: Some(1),
                stdout: r#"{"additionalContext": "should not appear"}"#.to_string(),
                stderr: String::new(),
            },
        )];
        let actual = UserHookHandler::<NullInfra>::collect_additional_context(&results);
        assert!(actual.is_empty());
    }
}
