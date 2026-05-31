use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_recursion::async_recursion;
use derive_setters::Setters;
use forge_domain::{Agent, *};
use forge_template::Element;
use futures::future::join_all;
use serde_json::json;
use tokio::sync::Notify;
use tracing::warn;

use crate::TemplateEngine;
use crate::agent::AgentService;
use crate::hooks::verification_reminder::{
    VERIFICATION_MATRIX_AGENT_NAME, background_refusal_reminder,
    background_refusal_reminder_was_sent, extract_verification_matrix_message,
    fallback_verification_matrix, has_any_tool_call, looks_like_refusal,
    verification_command_reminder, verification_command_reminder_was_sent,
    verification_command_was_run_after_skill, verification_gate_applies, verification_matrix_task,
    verification_matrix_was_sent, verification_reminder, verification_reminder_was_sent,
    verification_skill_was_called,
};
use crate::transformers::{DropReasoningOnlyMessages, ModelSpecificReasoning};

const MID_TIME_BUDGET_WARNING_FRACTION: f64 = 0.50;
const LOW_TIME_BUDGET_WARNING_FRACTION: f64 = 0.30;
const CRITICAL_TIME_BUDGET_WARNING_FRACTION: f64 = 0.20;

#[derive(Clone, Setters)]
#[setters(into)]
pub struct Orchestrator<S> {
    services: Arc<S>,
    sender: Option<ArcSender>,
    conversation: Conversation,
    environment: Environment,
    tool_definitions: Vec<ToolDefinition>,
    models: Vec<Model>,
    agent: Agent,
    error_tracker: ToolErrorTracker,
    hook: Arc<Hook>,
    retry_config: forge_config::RetryConfig,
    /// Optional task timeout in seconds, used for low-time budget warnings.
    task_timeout_secs: Option<u64>,
}

impl<S: AgentService> Orchestrator<S> {
    pub fn new(
        services: Arc<S>,
        environment: Environment,
        conversation: Conversation,
        agent: Agent,
        retry_config: forge_config::RetryConfig,
    ) -> Self {
        Self {
            task_timeout_secs: None,
            services,
            sender: Default::default(),
            conversation,
            environment,
            tool_definitions: Default::default(),
            models: Default::default(),
            agent,
            error_tracker: Default::default(),
            hook: Arc::new(crate::hooks::default()),
            retry_config,
        }
    }

    fn remaining_budget_fraction(&self, tool_context: &ToolCallContext) -> Option<f64> {
        let timeout = self.task_timeout_secs?;
        if timeout == 0 {
            return Some(0.0);
        }

        let fraction = tool_context
            .with_metrics(|m| {
                m.duration(chrono::Utc::now())
                    .map(|elapsed| {
                        let remaining = timeout as f64 - elapsed.as_secs_f64();
                        remaining / timeout as f64
                    })
                    .unwrap_or(1.0)
            })
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);

        Some(fraction)
    }

    fn remaining_budget_secs(&self, remaining_fraction: f64) -> Option<u64> {
        self.task_timeout_secs
            .map(|timeout| (remaining_fraction * timeout as f64).max(0.0) as u64)
    }

    async fn generate_verification_matrix(
        &self,
        tool_context: &ToolCallContext,
        context: &Context,
    ) -> Option<String> {
        let task = verification_matrix_task(context)?;
        let gate_agent = self
            .agent
            .clone()
            .tools(vec![ToolName::new(VERIFICATION_MATRIX_AGENT_NAME)]);
        let call = ToolCallFull::new(VERIFICATION_MATRIX_AGENT_NAME)
            .arguments(ToolCallArguments::from(json!({ "tasks": [task] })));
        let result = self.services.call(&gate_agent, tool_context, call).await;
        if result.is_error() {
            return fallback_verification_matrix(context);
        }

        let Some(raw_output) = result.output.as_str() else {
            return fallback_verification_matrix(context);
        };
        if looks_like_refusal(raw_output) {
            return fallback_verification_matrix(context);
        }

        extract_verification_matrix_message(raw_output)
            .or_else(|| fallback_verification_matrix(context))
    }

    pub fn get_conversation(&self) -> &Conversation {
        &self.conversation
    }

    // Helper function to get all tool results from a vector of tool calls
    #[async_recursion]
    async fn execute_tool_calls<'a>(
        &mut self,
        tool_calls: &[ToolCallFull],
        tool_context: &ToolCallContext,
    ) -> anyhow::Result<Vec<(ToolCallFull, ToolResult)>> {
        if self.environment.background {
            // In background mode: execute all tool calls concurrently, suppress
            // ToolCallStart/ToolCallEnd events and skip the UI notifier handshake
            // (there is no UI consumer in background mode, so awaiting the notifier
            // would deadlock).
            let futures: Vec<_> = tool_calls
                .iter()
                .map(|tool_call| {
                    let services = self.services.clone();
                    let agent = self.agent.clone();
                    let tool_context = tool_context.clone();
                    let tool_call = tool_call.clone();
                    async move {
                        let tool_result = services
                            .call(&agent, &tool_context, tool_call.clone())
                            .await;
                        (tool_call, tool_result)
                    }
                })
                .collect();

            let results = join_all(futures).await;

            // Fire lifecycle hooks sequentially after all parallel calls complete
            // (hooks mutate self.conversation so they cannot run in parallel).
            let mut tool_call_records = Vec::with_capacity(results.len());
            for (tool_call, tool_result) in results {
                let toolcall_start_event = LifecycleEvent::ToolcallStart(EventData::new(
                    self.agent.clone(),
                    self.agent.model.clone(),
                    ToolcallStartPayload::new(tool_call.clone()),
                ));
                self.hook
                    .handle(&toolcall_start_event, &mut self.conversation)
                    .await?;

                let toolcall_end_event = LifecycleEvent::ToolcallEnd(EventData::new(
                    self.agent.clone(),
                    self.agent.model.clone(),
                    ToolcallEndPayload::new(tool_call.clone(), tool_result.clone()),
                ));
                self.hook
                    .handle(&toolcall_end_event, &mut self.conversation)
                    .await?;

                tool_call_records.push((tool_call, tool_result));
            }

            return Ok(tool_call_records);
        }

        // Interactive mode: execute tool calls sequentially so the UI can render
        // each tool header before execution begins, and receive ordered events.
        let mut tool_call_records = Vec::with_capacity(tool_calls.len());

        let system_tools = self
            .tool_definitions
            .iter()
            .map(|tool| &tool.name)
            .collect::<HashSet<_>>();

        for tool_call in tool_calls {
            let is_system_tool = system_tools.contains(&tool_call.name);
            if is_system_tool {
                let notifier = Arc::new(Notify::new());
                self.send(ChatResponse::ToolCallStart {
                    tool_call: tool_call.clone(),
                    notifier: notifier.clone(),
                })
                .await?;
                // Wait for the UI to acknowledge it has rendered the tool header
                // before we execute the tool. This prevents tool stdout from
                // appearing before the tool name is printed.
                notifier.notified().await;
            }

            let toolcall_start_event = LifecycleEvent::ToolcallStart(EventData::new(
                self.agent.clone(),
                self.agent.model.clone(),
                ToolcallStartPayload::new(tool_call.clone()),
            ));
            self.hook
                .handle(&toolcall_start_event, &mut self.conversation)
                .await?;

            // Execute the tool
            let tool_result = self
                .services
                .call(&self.agent, tool_context, tool_call.clone())
                .await;

            // Fire the ToolcallEnd lifecycle event (fires on both success and failure)
            let toolcall_end_event = LifecycleEvent::ToolcallEnd(EventData::new(
                self.agent.clone(),
                self.agent.model.clone(),
                ToolcallEndPayload::new(tool_call.clone(), tool_result.clone()),
            ));
            self.hook
                .handle(&toolcall_end_event, &mut self.conversation)
                .await?;

            let is_system_tool = system_tools.contains(&tool_call.name);
            if is_system_tool {
                self.send(ChatResponse::ToolCallEnd(tool_result.clone()))
                    .await?;
            }

            tool_call_records.push((tool_call.clone(), tool_result));
        }

        Ok(tool_call_records)
    }

    async fn send(&self, message: ChatResponse) -> anyhow::Result<()> {
        if let Some(sender) = &self.sender {
            sender.send(Ok(message)).await?
        }
        Ok(())
    }

    // Returns if agent supports tool or not.
    fn is_tool_supported(&self) -> anyhow::Result<bool> {
        let model_id = &self.agent.model;

        // Check if at agent level tool support is defined
        let tool_supported = match self.agent.tool_supported {
            Some(tool_supported) => tool_supported,
            None => {
                // If not defined at agent level, check model level

                let model = self.models.iter().find(|model| &model.id == model_id);
                model
                    .and_then(|model| model.tools_supported)
                    .unwrap_or_default()
            }
        };

        Ok(tool_supported)
    }

    async fn execute_chat_turn(
        &self,
        model_id: &ModelId,
        context: Context,
        reasoning_supported: bool,
    ) -> anyhow::Result<ChatCompletionMessageFull> {
        let tool_supported = self.is_tool_supported()?;
        let mut transformers = DefaultTransformation::default()
            .pipe(SortTools::new(self.agent.tool_order()))
            .pipe(TransformToolCalls::new().when(|_| !tool_supported))
            .pipe(ImageHandling::new())
            .pipe(DocumentHandling::new())
            .pipe(DropReasoningDetails.when(|_| !reasoning_supported))
            // Strip all reasoning from messages when the model has changed (signatures are
            // model-specific and invalid across models). No-op when model is unchanged.
            .pipe(ReasoningNormalizer::new(model_id.clone()))
            // Normalize Anthropic reasoning knobs per model family before provider conversion.
            .pipe(
                ModelSpecificReasoning::new(model_id.as_str())
                    .when(|_| model_id.as_str().to_lowercase().contains("claude")),
            )
            // Drop reasoning-only assistant turns; Anthropic and Bedrock both reject
            // messages whose final content block is `thinking`.
            .pipe(
                DropReasoningOnlyMessages
                    .when(|_| model_id.as_str().to_lowercase().contains("claude")),
            );
        let response = self
            .services
            .chat_agent(
                model_id,
                transformers.transform(context),
                Some(self.agent.provider.clone()),
            )
            .await?;

        // Always stream content deltas
        response
            .into_full_streaming(!tool_supported, self.sender.clone())
            .await
    }

    // Create a helper method with the core functionality
    pub async fn run(&mut self) -> anyhow::Result<()> {
        let model_id = self.get_model();

        let mut context = self.conversation.context.clone().unwrap_or_default();

        // Fire the Start lifecycle event
        let start_event = LifecycleEvent::Start(EventData::new(
            self.agent.clone(),
            model_id.clone(),
            StartPayload,
        ));
        self.hook
            .handle(&start_event, &mut self.conversation)
            .await?;

        // Signals that the loop should suspend (task may or may not be completed)
        let mut should_yield = false;

        // Signals that the task is completed
        let mut is_complete = false;

        let mut request_count = 0;
        let mut mid_time_warning_sent = false;
        let mut low_time_warning_sent = false;
        let mut critical_time_warning_sent = false;

        // Retrieve the number of requests allowed per tick.
        let max_requests_per_turn = self.agent.max_requests_per_turn;

        let tool_context = ToolCallContext::new(self.conversation.metrics.clone())
            .sender(self.sender.clone())
            .conversation_id(self.conversation.id);

        while !should_yield {
            // Set context for the current loop iteration
            self.conversation.context = Some(context.clone());
            self.services.update(self.conversation.clone()).await?;

            // Fire the Request lifecycle event
            let request_event = LifecycleEvent::Request(EventData::new(
                self.agent.clone(),
                model_id.clone(),
                RequestPayload::new(request_count),
            ));
            self.hook
                .handle(&request_event, &mut self.conversation)
                .await?;

            let message = crate::retry::retry_with_config(
                &self.retry_config,
                || {
                    self.execute_chat_turn(
                        &model_id,
                        context.clone(),
                        context.is_reasoning_supported(),
                    )
                },
                self.sender.as_ref().map(|sender| {
                    let sender = sender.clone();
                    let agent_id = self.agent.id.clone();
                    let model_id = model_id.clone();
                    move |error: &anyhow::Error, duration: Duration| {
                        let root_cause = error.root_cause();
                        // Log retry attempts - critical for debugging API failures
                        tracing::error!(
                            agent_id = %agent_id,
                            error = ?root_cause,
                            model = %model_id,
                            "Retry attempt due to error"
                        );
                        let retry_event =
                            ChatResponse::RetryAttempt { cause: error.into(), duration };
                        let _ = sender.try_send(Ok(retry_event));
                    }
                }),
            )
            .await?;

            // Fire the Response lifecycle event
            let response_event = LifecycleEvent::Response(EventData::new(
                self.agent.clone(),
                model_id.clone(),
                ResponsePayload::new(message.clone()),
            ));
            self.hook
                .handle(&response_event, &mut self.conversation)
                .await?;

            // Turn is completed, if finish_reason is 'stop'. Gemini models return stop as
            // finish reason with tool calls.
            is_complete =
                message.finish_reason == Some(FinishReason::Stop) && message.tool_calls.is_empty();

            // Should yield if a tool is asking for a follow-up
            should_yield = is_complete
                || message
                    .tool_calls
                    .iter()
                    .any(|call| ToolCatalog::should_yield(&call.name));

            // Process tool calls and update context
            let mut tool_call_records = self
                .execute_tool_calls(&message.tool_calls, &tool_context)
                .await?;

            // Update context from conversation after tool-call hooks run
            if let Some(updated_context) = &self.conversation.context {
                context = updated_context.clone();
            }

            self.error_tracker.adjust_record(&tool_call_records);
            let allowed_max_attempts = self.error_tracker.limit();
            for (_, result) in tool_call_records.iter_mut() {
                if result.is_error() {
                    let attempts_left = self.error_tracker.remaining_attempts(&result.name);
                    // Add attempt information to the error message so the agent can reflect on it.
                    let context = serde_json::json!({
                        "attempts_left": attempts_left,
                        "allowed_max_attempts": allowed_max_attempts,
                    });
                    let text = TemplateEngine::default()
                        .render("forge-tool-retry-message.md", &context)?;
                    let message = Element::new("retry").text(text);

                    result.output.combine_mut(ToolOutput::text(message));
                }
            }

            // Proactively generate the verification-matrix when the agent calls
            // verification-specialist for the first time, and attach it to the
            // skill's tool result so the agent sees it immediately.
            if self.environment.background
                && verification_gate_applies(&self.agent, &self.tool_definitions)
                && !verification_matrix_was_sent(&context)
            {
                let vs_called_this_turn = tool_call_records.iter().any(|(call, _)| {
                    ToolCatalog::try_from(call.clone())
                        .ok()
                        .is_some_and(|tool| matches!(tool, ToolCatalog::Skill(ref s) if s.name == "verification-specialist"))
                });
                if vs_called_this_turn
                    && let Some(matrix) = self
                        .generate_verification_matrix(&tool_context, &context)
                        .await
                {
                    // Append the matrix to the first verification-specialist
                    // tool result so the agent receives it inline.
                    for (call, result) in tool_call_records.iter_mut() {
                        let is_vs = ToolCatalog::try_from(call.clone())
                            .ok()
                            .is_some_and(|tool| matches!(tool, ToolCatalog::Skill(ref s) if s.name == "verification-specialist"));
                        if is_vs {
                            result
                                .output
                                .combine_mut(ToolOutput::text(format!("\n\n{matrix}")));
                            break;
                        }
                    }
                }
            }

            // Time-budget reminders: progressively shift behavior toward
            // artifact-first completion as budget gets tighter.
            if !is_complete
                && let Some(remaining_fraction) = self.remaining_budget_fraction(&tool_context)
                && let Some(timeout) = self.task_timeout_secs
            {
                if remaining_fraction <= CRITICAL_TIME_BUDGET_WARNING_FRACTION
                    && !critical_time_warning_sent
                {
                    critical_time_warning_sent = true;
                    let remaining_secs = self
                        .remaining_budget_secs(remaining_fraction)
                        .unwrap_or_default();
                    let warning = Element::new("system-warning")
                        .attr("type", "critical-time-budget")
                        .text(format!(
                            "URGENT: Only ~{remaining_secs}s remaining out of {timeout}s budget. \
                             Save your deliverables NOW, then call verification-specialist. \
                             Do not start new implementation work."
                        ));
                    context = context.add_message(ContextMessage::user(warning.to_string(), None));
                    should_yield = false;
                    is_complete = false;
                } else if remaining_fraction <= LOW_TIME_BUDGET_WARNING_FRACTION
                    && !low_time_warning_sent
                {
                    low_time_warning_sent = true;
                    let remaining_secs = self
                        .remaining_budget_secs(remaining_fraction)
                        .unwrap_or_default();
                    let warning = Element::new("system-warning")
                        .attr("type", "low-time-budget")
                        .text(format!(
                            "Low time budget: ~{remaining_secs}s remaining out of {timeout}s. \
                             Stop exploratory loops and new dependency bootstrapping. \
                             Finalize required artifacts now and run one direct smoke verification."
                        ));
                    context = context.add_message(ContextMessage::user(warning.to_string(), None));
                    should_yield = false;
                    is_complete = false;
                } else if remaining_fraction <= MID_TIME_BUDGET_WARNING_FRACTION
                    && !mid_time_warning_sent
                {
                    mid_time_warning_sent = true;
                    let remaining_secs = self
                        .remaining_budget_secs(remaining_fraction)
                        .unwrap_or_default();
                    let warning = Element::new("system-warning")
                        .attr("type", "time-budget-checkpoint")
                        .text(format!(
                            "Budget checkpoint: ~{remaining_secs}s remaining out of {timeout}s. \
                             Prefer the shortest path to the required deliverables. \
                             Avoid broad exploration or repeated setup work unless strictly required."
                        ));
                    context = context.add_message(ContextMessage::user(warning.to_string(), None));
                    should_yield = false;
                    is_complete = false;
                }
            }

            let looks_like_refusal_message = looks_like_refusal(&message.content);
            let refusal_recovery_needed = self.environment.background
                && is_complete
                && message.tool_calls.is_empty()
                && !background_refusal_reminder_was_sent(&context)
                && !has_any_tool_call(&context)
                && looks_like_refusal_message;

            context = context.append_message(
                message.content.clone(),
                message.thought_signature.clone(),
                message.reasoning.clone(),
                message.reasoning_details.clone(),
                message.usage,
                tool_call_records,
                message.phase,
            );

            if is_complete
                && !context.messages.iter().any(|msg| {
                    msg.content()
                        .is_some_and(|content| content.contains("pending todo items"))
                })
            {
                let pending_todos = self
                    .services
                    .get_pending_todos(&self.conversation.id)
                    .await?;
                if !pending_todos.is_empty() {
                    let reminder = format!(
                        "You have {} pending todo items. Please complete them before finishing the task.",
                        pending_todos.len()
                    );
                    context = context.add_message(ContextMessage::user(reminder, None));
                    should_yield = false;
                    is_complete = false;
                }
            }

            if refusal_recovery_needed {
                context =
                    context.add_message(ContextMessage::user(background_refusal_reminder(), None));
                should_yield = false;
                is_complete = false;
            }

            let verification_gate_enabled = self.environment.background
                && verification_gate_applies(&self.agent, &self.tool_definitions);
            let verification_reminder_already_sent = verification_reminder_was_sent(&context);
            let verification_command_reminder_already_sent =
                verification_command_reminder_was_sent(&context);
            let verification_matrix_already_sent = verification_matrix_was_sent(&context);

            if verification_gate_enabled
                && is_complete
                && !verification_reminder_already_sent
                && !verification_skill_was_called(&context)
            {
                let matrix = if !verification_matrix_already_sent {
                    self.generate_verification_matrix(&tool_context, &context)
                        .await
                } else {
                    None
                };
                context = context.add_message(ContextMessage::user(
                    verification_reminder(matrix.as_deref()),
                    None,
                ));
                should_yield = false;
                is_complete = false;
            }

            if verification_gate_enabled
                && is_complete
                && verification_skill_was_called(&context)
                && !verification_command_reminder_already_sent
                && !verification_command_was_run_after_skill(&context)
            {
                let matrix = if !verification_matrix_already_sent {
                    self.generate_verification_matrix(&tool_context, &context)
                        .await
                } else {
                    None
                };
                context = context.add_message(ContextMessage::user(
                    verification_command_reminder(matrix.as_deref()),
                    None,
                ));
                should_yield = false;
                is_complete = false;
            }

            if self.error_tracker.limit_reached() {
                self.send(ChatResponse::Interrupt {
                    reason: InterruptionReason::MaxToolFailurePerTurnLimitReached {
                        limit: *self.error_tracker.limit() as u64,
                        errors: self.error_tracker.errors().clone(),
                    },
                })
                .await?;
                // Should yield if too many errors are produced
                should_yield = true;
            }

            // Update context in the conversation
            context = SetModel::new(model_id.clone()).transform(context);
            self.conversation.context = Some(context.clone());
            self.services.update(self.conversation.clone()).await?;
            request_count += 1;

            if !should_yield && let Some(max_request_allowed) = max_requests_per_turn {
                // Check if agent has reached the maximum request per turn limit
                if request_count >= max_request_allowed {
                    // Log warning - important for understanding conversation interruptions
                    warn!(
                        agent_id = %self.agent.id,
                        model_id = %model_id,
                        request_count,
                        max_request_allowed,
                        "Agent has reached the maximum request per turn limit"
                    );
                    // raise an interrupt event to notify the UI
                    self.send(ChatResponse::Interrupt {
                        reason: InterruptionReason::MaxRequestPerTurnLimitReached {
                            limit: max_request_allowed as u64,
                        },
                    })
                    .await?;
                    // force completion
                    should_yield = true;
                }
            }

            // Update metrics in conversation
            tool_context.with_metrics(|metrics| {
                self.conversation.metrics = metrics.clone();
            })?;
        }

        // Fire the End lifecycle event (title will be set here by the hook)
        self.hook
            .handle(
                &LifecycleEvent::End(EventData::new(
                    self.agent.clone(),
                    model_id.clone(),
                    EndPayload,
                )),
                &mut self.conversation,
            )
            .await?;

        self.services.update(self.conversation.clone()).await?;

        // Signal Task Completion
        if is_complete {
            self.send(ChatResponse::TaskComplete).await?;
        }

        Ok(())
    }

    fn get_model(&self) -> ModelId {
        self.agent.model.clone()
    }
}
