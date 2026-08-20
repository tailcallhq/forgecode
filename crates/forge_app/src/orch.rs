use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_recursion::async_recursion;
use derive_setters::Setters;
use forge_domain::{Agent, *};
use forge_template::Element;
use futures::future::join_all;
use tokio::sync::Notify;
use tracing::warn;

use crate::agent::AgentService;
use crate::transformers::{DropReasoningOnlyMessages, ModelSpecificReasoning};
use crate::{EnvironmentInfra, TemplateEngine};

#[derive(Clone, Setters)]
#[setters(into)]
pub struct Orchestrator<S> {
    services: Arc<S>,
    sender: Option<ArcSender>,
    conversation: Conversation,
    tool_definitions: Vec<ToolDefinition>,
    models: Vec<Model>,
    agent: Agent,
    error_tracker: ToolErrorTracker,
    hook: Arc<Hook>,
    config: forge_config::ForgeConfig,
    dirty: bool,
    /// Pluggable telemetry sink — no-op by default, zero overhead unless
    /// replaced with a real implementation via `.metrics_sink(sink)`.
    #[setters(skip)]
    metrics_sink: Arc<dyn MetricsSink>,
}

impl<S: AgentService + EnvironmentInfra<Config = forge_config::ForgeConfig>> Orchestrator<S> {
    pub fn new(
        services: Arc<S>,
        conversation: Conversation,
        agent: Agent,
        config: forge_config::ForgeConfig,
    ) -> Self {
        Self {
            conversation,
            services,
            agent,
            config,
            sender: Default::default(),
            tool_definitions: Default::default(),
            models: Default::default(),
            error_tracker: Default::default(),
            hook: Arc::new(Hook::default()),
            dirty: false,
            metrics_sink: Arc::new(NoopMetricsSink),
        }
    }

    /// Replace the no-op telemetry sink with a real implementation.
    ///
    /// Call this once during setup; the orchestrator keeps an `Arc` so the sink
    /// can be shared cheaply across clones.
    // Public injection point for a real metrics sink, supplied by embedders; no
    // internal caller yet. Justified suppression: it is the only wiring for P2
    // observability and removing it would drop that capability.
    #[allow(dead_code)]
    pub fn with_metrics_sink(mut self, sink: Arc<dyn MetricsSink>) -> Self {
        self.metrics_sink = sink;
        self
    }

    /// Get a reference to the internal conversation
    pub fn get_conversation(&self) -> &Conversation {
        &self.conversation
    }

    // Helper function to get all tool results from a vector of tool calls
    #[tracing::instrument(skip(self, tool_calls, tool_context), fields(tool_count = tool_calls.len()))]
    #[async_recursion]
    async fn execute_tool_calls(
        &mut self,
        tool_calls: &[ToolCallFull],
        tool_context: &ToolCallContext,
    ) -> anyhow::Result<Vec<(ToolCallFull, ToolResult)>> {
        let task_tool_name = ToolKind::Task.name();

        // Use a case-insensitive comparison since the model may send "Task" or "task".
        let is_task = |tc: &ToolCallFull| {
            tc.name
                .as_str()
                .eq_ignore_ascii_case(task_tool_name.as_str())
        };

        // Partition into task tool calls (run in parallel) and all others (run
        // sequentially). Use a case-insensitive comparison since the model may
        // send "Task" or "task".
        let is_task_call =
            |tc: &&ToolCallFull| tc.name.as_str().to_lowercase() == task_tool_name.as_str();
        let (task_calls, other_calls): (Vec<_>, Vec<_>) = tool_calls.iter().partition(is_task_call);

        // Execute task tool calls in parallel — mirrors how direct agent-as-tool calls
        // work.
        let task_results: Vec<(ToolCallFull, ToolResult)> = join_all(
            task_calls
                .iter()
                .map(|tc| self.services.call(&self.agent, tool_context, (*tc).clone())),
        )
        .await
        .into_iter()
        .zip(task_calls.iter())
        .map(|(result, tc)| ((*tc).clone(), result))
        .collect();

        let system_tools = self
            .tool_definitions
            .iter()
            .map(|tool| &tool.name)
            .collect::<HashSet<_>>();

        // Process non-task tool calls sequentially (preserving UI notifier handshake
        // and hooks).
        let mut other_results: Vec<(ToolCallFull, ToolResult)> =
            Vec::with_capacity(other_calls.len());
        for tool_call in &other_calls {
            // Send the start notification for system tools and not agent as a tool
            let is_system_tool = system_tools.contains(&tool_call.name);
            if is_system_tool {
                let notifier = Arc::new(Notify::new());
                self.send(ChatResponse::ToolCallStart {
                    tool_call: (*tool_call).clone(),
                    notifier: notifier.clone(),
                })
                .await?;
                // Wait for the UI to acknowledge it has rendered the tool header
                // before we execute the tool. This prevents tool stdout from
                // appearing before the tool name is printed.
                notifier.notified().await;
            }

            // Fire the ToolcallStart lifecycle event
            let toolcall_start_event = LifecycleEvent::ToolcallStart(EventData::new(
                self.agent.clone(),
                self.agent.model.clone(),
                ToolcallStartPayload::new((*tool_call).clone()),
            ));
            self.hook
                .handle(&toolcall_start_event, &mut self.conversation)
                .await?;

            // Execute the tool
            let tool_result = self
                .services
                .call(&self.agent, tool_context, (*tool_call).clone())
                .await;

            // Fire the ToolcallEnd lifecycle event (fires on both success and failure)
            let toolcall_end_event = LifecycleEvent::ToolcallEnd(EventData::new(
                self.agent.clone(),
                self.agent.model.clone(),
                ToolcallEndPayload::new((*tool_call).clone(), tool_result.clone()),
            ));
            self.hook
                .handle(&toolcall_end_event, &mut self.conversation)
                .await?;

            // Send the end notification for system tools and not agent as a tool
            if is_system_tool {
                self.send(ChatResponse::ToolCallEnd(tool_result.clone()))
                    .await?;
            }
            other_results.push(((*tool_call).clone(), tool_result));
        }

        // Reconstruct results in the original order of tool_calls.
        let mut task_iter = task_results.into_iter();
        let mut other_iter = other_results.into_iter();
        let tool_call_records = tool_calls
            .iter()
            .map(|tc| {
                if is_task(tc) {
                    task_iter.next().expect("task result count mismatch")
                } else {
                    other_iter.next().expect("other result count mismatch")
                }
            })
            .collect();

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

    #[tracing::instrument(skip(self, context), fields(model = %model_id, reasoning = reasoning_supported))]
    async fn execute_chat_turn(
        &self,
        model_id: &ModelId,
        context: Context,
        reasoning_supported: bool,
    ) -> anyhow::Result<ChatCompletionMessageFull> {
        let tool_supported = self.is_tool_supported()?;
        let mut transformers = DefaultTransformation::default()
            .pipe(SortTools::new(self.agent.tool_order()))
            .pipe(NormalizeToolCallArguments::new())
            .pipe(TransformToolCalls::new().when(|_| !tool_supported))
            .pipe(ImageHandling::new())
            // Drop ALL reasoning (including config) when reasoning is not supported by the model
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
    #[tracing::instrument(skip(self), fields(agent = %self.agent.id, conversation = %self.conversation.id))]
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

        // Install crash-safety guard: if `run` exits via panic or cancellation,
        // the guard's `Drop` performs a best-effort final persist via
        // `services.update`. Held for the entire body of `run`. The guard owns
        // a snapshot of the data it needs (instead of borrowing `self`) so the
        // rest of `run` can keep using `self.foo()` without conflicts.
        let mut _drop_guard = OrchestratorDropGuard {
            dirty: self.dirty,
            conversation: Some(self.conversation.clone()),
            services: self.services.clone(),
        };

        let mut request_count = 0;

        // Retrieve the number of requests allowed per tick.
        let max_requests_per_turn = self.agent.max_requests_per_turn;
        let tool_context = {
            let mut ctx =
                ToolCallContext::new(self.conversation.metrics.clone()).sender(self.sender.clone());
            ctx.set_conversation_id(Some(self.conversation.id));
            ctx.set_parent_id(self.conversation.parent_id);
            ctx.set_source(self.conversation.source.clone());
            ctx
        };

        while !should_yield {
            // Set context for the current loop iteration
            self.conversation.context = Some(context.clone());
            self.mark_dirty();
            self.flush_if_dirty().await?;

            let request_event = LifecycleEvent::Request(EventData::new(
                self.agent.clone(),
                model_id.clone(),
                RequestPayload::new(request_count),
            ));
            self.hook
                .handle(&request_event, &mut self.conversation)
                .await?;

            // Telemetry: count each outgoing request
            self.metrics_sink.increment(metric_names::REQUEST, 1);

            let message = crate::retry::retry_with_config(
                &self.config.clone().retry.unwrap_or_default(),
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
                    let metrics_sink = self.metrics_sink.clone();
                    move |error: &anyhow::Error, duration: Duration| {
                        let root_cause = error.root_cause();
                        // Log retry attempts - critical for debugging API failures
                        tracing::error!(
                            agent_id = %agent_id,
                            error = ?root_cause,
                            model = %model_id,
                            "Retry attempt due to error"
                        );
                        metrics_sink.increment(metric_names::RETRY, 1);
                        let retry_event =
                            ChatResponse::RetryAttempt { cause: error.into(), duration };
                        let _ = sender.try_send(Ok(retry_event));
                    }
                }),
            )
            .await?;

            // Telemetry: model execution completed
            self.metrics_sink.increment(metric_names::MODEL_EXEC, 1);

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

            // Update context from conversation after response / tool-call hooks run
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

            context = context.append_message(
                message.content.clone(),
                message.thought_signature.clone(),
                message.reasoning.clone(),
                message.reasoning_details.clone(),
                message.usage,
                tool_call_records,
                message.phase,
            );

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
            self.mark_dirty();
            self.flush_if_dirty().await?;
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

            // If completing (should_yield is due), fire End hook and check if
            // it adds messages
            if should_yield {
                let end_count_before = self.conversation.len();
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
                self.mark_dirty();
                self.flush_if_dirty().await?;
                // Check if End hook added messages - if so, continue the loop
                if self.conversation.len() > end_count_before {
                    // End hook added messages, sync context and continue
                    if let Some(updated_context) = &self.conversation.context {
                        context = updated_context.clone();
                    }
                    should_yield = false;
                }
            }
        }

        self.mark_dirty();
        self.flush_if_dirty().await?;

        // Signal Task Completion
        if is_complete {
            self.send(ChatResponse::TaskComplete).await?;
        }

        Ok(())
    }

    fn get_model(&self) -> ModelId {
        self.agent.model.clone()
    }

    /// Mark the conversation as dirty so the next `flush_if_dirty` will
    /// persist. Cheap (no I/O) — call whenever the conversation changes.
    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Persist the conversation if `dirty` is set, then clear the flag. This is
    /// the single chokepoint where `services.update` is called from `run`,
    /// paired with `OrchestratorDropGuard` for crash-safety on
    /// panic/cancellation.
    async fn flush_if_dirty(&mut self) -> anyhow::Result<()> {
        if self.dirty {
            self.services.update(self.conversation.clone()).await?;
            self.dirty = false;
        }
        Ok(())
    }
}

/// Crash-safety guard for `Orchestrator::run`. If `run` exits via panic or
/// cancellation before `flush_if_dirty` clears the dirty flag, the `Drop` impl
/// performs a best-effort final `services.update`. Stores only the data needed
/// for the final persist so it does not borrow the orchestrator and conflict
/// with the rest of `run`'s `self.foo()` calls.
struct OrchestratorDropGuard<S>
where
    S: AgentService + EnvironmentInfra<Config = forge_config::ForgeConfig>,
{
    dirty: bool,
    conversation: Option<crate::domain::Conversation>,
    services: Arc<S>,
}

impl<S> Drop for OrchestratorDropGuard<S>
where
    S: AgentService + EnvironmentInfra<Config = forge_config::ForgeConfig>,
{
    fn drop(&mut self) {
        // Best-effort final persist on panic/cancellation. Uses block_in_place
        // because Drop cannot be async; the underlying SQLite write is fast
        // (a single statement), so this is acceptable.
        if self.dirty
            && let Some(conversation) = self.conversation.take()
        {
            let services = self.services.clone();
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    let _ = services.update(conversation).await;
                })
            });
        }
    }
}
