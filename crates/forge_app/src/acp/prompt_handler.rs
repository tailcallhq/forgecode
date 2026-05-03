use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use agent_client_protocol as acp;
use agent_client_protocol::Client;
use forge_config::ForgeConfig;
use forge_domain::{
    ChatRequest, ChatResponse, ChatResponseContent, Event, EventValue, InterruptionReason,
};
use futures::StreamExt;
use tokio::sync::Notify;

use crate::{EnvironmentInfra, ForgeApp, Services};

use super::adapter::AcpAdapter;
use super::conversion;
use super::error::{self, Error, Result};

impl<S: Services + EnvironmentInfra<Config = ForgeConfig>> AcpAdapter<S> {
    pub(super) async fn handle_prompt(
        &self,
        arguments: acp::PromptRequest,
    ) -> std::result::Result<acp::PromptResponse, acp::Error> {
        let session_key = arguments.session_id.0.as_ref().to_string();
        let session = self.session_state(&session_key).await.map_err(error::into_acp_error)?;

        let mut prompt_text_parts = Vec::new();
        let mut attachments = Vec::new();

        for content_block in &arguments.prompt {
            match content_block {
                acp::ContentBlock::Text(text_content) => {
                    prompt_text_parts.push(text_content.text.clone());
                }
                acp::ContentBlock::ResourceLink(resource_link) => {
                    let path = conversion::uri_to_path(&resource_link.uri);
                    prompt_text_parts.push(format!("@[{}]", path));
                }
                acp::ContentBlock::Resource(embedded_resource) => {
                    match conversion::acp_resource_to_attachment(embedded_resource) {
                        Ok(attachment) => attachments.push(attachment),
                        Err(error) => {
                            tracing::warn!("Failed to convert embedded resource: {}", error);
                        }
                    }
                }
                _ => {}
            }
        }

        let prompt_text = prompt_text_parts.join("\n");
        let cancel_notify = Arc::new(Notify::new());
        let cancelled = Arc::new(AtomicBool::new(false));
        self.set_cancel_notify(&session_key, Some(cancel_notify.clone()))
            .await
            .map_err(error::into_acp_error)?;

        let response = self
            .run_prompt_loop(
                &arguments.session_id,
                &session_key,
                session,
                prompt_text,
                attachments,
                cancel_notify,
                cancelled,
            )
            .await;

        let _ = self.set_cancel_notify(&session_key, None).await;
        response
    }

    async fn run_prompt_loop(
        &self,
        session_id: &acp::SessionId,
        session_key: &str,
        session: super::adapter::SessionState,
        prompt_text: String,
        attachments: Vec<forge_domain::Attachment>,
        cancel_notify: Arc<Notify>,
        cancelled: Arc<AtomicBool>,
    ) -> std::result::Result<acp::PromptResponse, acp::Error> {
        let mut event = Event::new(EventValue::text(prompt_text));
        event.attachments = attachments;

        let mut chat_request = ChatRequest::new(event, session.conversation_id);
        loop {
            // Check if cancellation was requested before starting a new
            // chat round (handles the case where cancel arrives between
            // loop iterations).
            if cancelled.load(Ordering::SeqCst) {
                tracing::info!("ACP prompt cancelled for session {}", session_key);
                return Ok(acp::PromptResponse::new(acp::StopReason::Cancelled));
            }

            let app = ForgeApp::new(self.services.clone());
            let mut stream = app
                .chat(session.agent_id.clone(), chat_request)
                .await
                .map_err(|error| acp::Error::into_internal_error(error.as_ref() as &dyn std::error::Error))?;

            let mut continue_after_interrupt = false;

            loop {
                tokio::select! {
                    _ = cancel_notify.notified() => {
                        cancelled.store(true, Ordering::SeqCst);
                        tracing::info!("ACP prompt cancelled for session {}", session_key);
                        return Ok(acp::PromptResponse::new(acp::StopReason::Cancelled));
                    }
                    response_result = stream.next() => {
                        match response_result {
                            Some(Ok(response)) => {
                                self.handle_chat_response(session_id, response, &mut continue_after_interrupt).await?;
                            }
                            Some(Err(error)) => {
                                tracing::error!("Error in chat stream: {}", error);
                                return Err(acp::Error::into_internal_error(
                                    error.as_ref() as &dyn std::error::Error,
                                ));
                            }
                            None => {
                                break;
                            }
                        }
                    }
                }
            }

            if continue_after_interrupt {
                chat_request = ChatRequest::new(Event::new(EventValue::text("")), session.conversation_id);
                continue;
            }

            return Ok(acp::PromptResponse::new(acp::StopReason::EndTurn));
        }
    }

    async fn handle_chat_response(
        &self,
        session_id: &acp::SessionId,
        response: ChatResponse,
        continue_after_interrupt: &mut bool,
    ) -> std::result::Result<(), acp::Error> {
        match response {
            ChatResponse::TaskMessage { content } => {
                self.handle_task_message(session_id, content).await?;
            }
            ChatResponse::TaskReasoning { content } => {
                if !content.is_empty() {
                    let notification = acp::SessionNotification::new(
                        session_id.clone(),
                        acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(
                            acp::ContentBlock::Text(acp::TextContent::new(content)),
                        )),
                    );
                    self.send_notification(notification)
                        .map_err(error::into_acp_error)?;
                }
            }
            ChatResponse::ToolCallStart { tool_call, .. } => {
                let notification = acp::SessionNotification::new(
                    session_id.clone(),
                    acp::SessionUpdate::ToolCallUpdate(
                        conversion::map_tool_call_to_acp(&tool_call).into(),
                    ),
                );
                self.send_notification(notification)
                    .map_err(error::into_acp_error)?;
            }
            ChatResponse::ToolCallEnd(tool_result) => {
                let content = conversion::convert_tool_output(&tool_result.output);
                let status = if tool_result.output.is_error {
                    acp::ToolCallStatus::Failed
                } else {
                    acp::ToolCallStatus::Completed
                };
                let tool_call_id = tool_result
                    .call_id
                    .as_ref()
                    .map(|id| id.as_str().to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                let update = acp::ToolCallUpdate::new(
                    tool_call_id,
                    acp::ToolCallUpdateFields::new().status(status).content(content),
                );
                let notification = acp::SessionNotification::new(
                    session_id.clone(),
                    acp::SessionUpdate::ToolCallUpdate(update),
                );
                self.send_notification(notification)
                    .map_err(error::into_acp_error)?;
            }
            ChatResponse::TaskComplete => {}
            ChatResponse::RetryAttempt { .. } => {}
            ChatResponse::Interrupt { reason } => {
                let should_continue = self
                    .request_continue_permission(session_id, &reason)
                    .await
                    .map_err(error::into_acp_error)?;
                if should_continue {
                    *continue_after_interrupt = true;
                }
            }
        }

        Ok(())
    }

    async fn handle_task_message(
        &self,
        session_id: &acp::SessionId,
        content: ChatResponseContent,
    ) -> std::result::Result<(), acp::Error> {
        match content {
            ChatResponseContent::ToolOutput(_) => {}
            ChatResponseContent::Markdown { text, .. } => {
                if !text.is_empty() {
                    let notification = acp::SessionNotification::new(
                        session_id.clone(),
                        acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                            acp::ContentBlock::Text(acp::TextContent::new(text)),
                        )),
                    );
                    self.send_notification(notification)
                        .map_err(error::into_acp_error)?;
                }
            }
            ChatResponseContent::ToolInput(_) => {}
        }

        Ok(())
    }

    async fn request_continue_permission(
        &self,
        session_id: &acp::SessionId,
        reason: &InterruptionReason,
    ) -> Result<bool> {
        let client_conn = self.client_conn.lock().await;
        let Some(conn) = client_conn.as_ref() else {
            return Ok(false);
        };

        let (title, description) = format_interruption(reason);
        let options = vec![
            acp::PermissionOption::new(
                "continue",
                "Continue Anyway",
                acp::PermissionOptionKind::AllowOnce,
            ),
            acp::PermissionOption::new("stop", "Stop", acp::PermissionOptionKind::RejectOnce),
        ];
        let tool_call_update = acp::ToolCallUpdate::new(
            "interrupt-continue",
            acp::ToolCallUpdateFields::new()
                .status(acp::ToolCallStatus::Pending)
                .title(title.clone()),
        );

        let mut request = acp::RequestPermissionRequest::new(
            session_id.clone(),
            tool_call_update,
            options,
        );
        let mut meta = serde_json::Map::new();
        meta.insert("title".to_string(), serde_json::json!(title));
        meta.insert("description".to_string(), serde_json::json!(description));
        request = request.meta(meta);

        let response = conn.request_permission(request).await.map_err(|error| {
            Error::Application(anyhow::anyhow!("Permission request failed: {}", error))
        })?;

        match response.outcome {
            acp::RequestPermissionOutcome::Selected(selection) => {
                Ok(selection.option_id.0.as_ref() == "continue")
            }
            acp::RequestPermissionOutcome::Cancelled => Ok(false),
            _ => Ok(false),
        }
    }
}

fn format_interruption(reason: &InterruptionReason) -> (String, String) {
    match reason {
        InterruptionReason::MaxToolFailurePerTurnLimitReached { limit, errors } => {
            let error_summary = errors
                .iter()
                .map(|(tool_name, count)| format!("{} ({})", tool_name, count))
                .collect::<Vec<_>>()
                .join(", ");
            (
                format!("Tool failure limit reached ({})", limit),
                format!("Forge stopped after repeated tool failures: {}", error_summary),
            )
        }
        InterruptionReason::MaxRequestPerTurnLimitReached { limit } => (
            format!("Request limit reached ({})", limit),
            "Forge reached the maximum number of requests for this turn.".to_string(),
        ),
    }
}
