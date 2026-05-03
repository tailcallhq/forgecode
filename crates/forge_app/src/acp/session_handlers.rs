use agent_client_protocol as acp;
use forge_config::ForgeConfig;
use forge_domain::{AgentId, ConfigOperation, Conversation, ConversationId, ModelConfig, ModelId};

use crate::{AgentRegistry, AppConfigService, ConversationService, EnvironmentInfra, Services};

use super::adapter::{AcpAdapter, SessionState};
use super::error;
use super::state_builders::StateBuilders;

const VERSION: &str = env!("CARGO_PKG_VERSION");

impl<S: Services + EnvironmentInfra<Config = ForgeConfig>> AcpAdapter<S> {
    pub(super) async fn handle_initialize(
        &self,
        arguments: acp::InitializeRequest,
    ) -> std::result::Result<acp::InitializeResponse, acp::Error> {
        tracing::info!("Received initialize request from client: {:?}", arguments.client_info);

        Ok(acp::InitializeResponse::new(acp::ProtocolVersion::V1)
            .agent_capabilities(
                acp::AgentCapabilities::new().load_session(true).mcp_capabilities(
                    acp::McpCapabilities::new()
                        .http(true)
                        .sse(true),
                ),
            )
            .agent_info(
                acp::Implementation::new("forge".to_string(), VERSION.to_string())
                    .title("Forge Code".to_string()),
            ))
    }

    /// Handles ACP authentication.
    ///
    /// This is intentionally a no-op. The stdio transport inherits OS-level
    /// process isolation: only the parent process (e.g. Acepe) that spawned
    /// `forge machine stdio` can read/write the stdin/stdout pipes. No
    /// network listener is opened, so no additional authentication is
    /// required. See `AcpApp::start_stdio` for the full trust model.
    pub(super) async fn handle_authenticate(
        &self,
        _arguments: acp::AuthenticateRequest,
    ) -> std::result::Result<acp::AuthenticateResponse, acp::Error> {
        tracing::debug!("ACP authenticate: no-op (stdio transport uses OS process isolation)");
        Ok(acp::AuthenticateResponse::default())
    }

    pub(super) async fn handle_new_session(
        &self,
        arguments: acp::NewSessionRequest,
    ) -> std::result::Result<acp::NewSessionResponse, acp::Error> {
        if !arguments.mcp_servers.is_empty() {
            StateBuilders::load_mcp_servers(self.services.as_ref(), &arguments.mcp_servers)
                .await
                .map_err(error::into_acp_error)?;
        }

        let active_agent_id = self
            .services
            .agent_registry()
            .get_active_agent_id()
            .await
            .map_err(|error| acp::Error::into_internal_error(&*error))?
            .unwrap_or_default();

        let conversation = Conversation::generate();
        let conversation_id = conversation.id;
        self.services
            .conversation_service()
            .upsert_conversation(conversation)
            .await
            .map_err(|error| acp::Error::into_internal_error(&*error))?;

        let session_id = acp::SessionId::new(conversation_id.into_string());
        let session_key = session_id.0.as_ref().to_string();
        self.store_session(
            session_key,
            SessionState {
                conversation_id,
                agent_id: active_agent_id.clone(),
                model_id: None,
                cancel_notify: None,
            },
        )
        .await;

        let agent = self
            .services
            .agent_registry()
            .get_agent(&active_agent_id)
            .await
            .map_err(|error| acp::Error::into_internal_error(&*error))?
            .ok_or_else(|| {
                acp::Error::into_internal_error(&*anyhow::anyhow!(
                    "Agent '{}' not found",
                    active_agent_id
                ))
            })?;

        let mode_state = StateBuilders::build_session_mode_state(
            self.services.as_ref(),
            &active_agent_id,
        )
        .await
        .map_err(error::into_acp_error)?;
        let model_state = StateBuilders::build_session_model_state(&self.services, &agent)
            .await
            .map_err(error::into_acp_error)?;

        Ok(acp::NewSessionResponse::new(session_id)
            .modes(mode_state)
            .models(model_state))
    }

    pub(super) async fn handle_load_session(
        &self,
        arguments: acp::LoadSessionRequest,
    ) -> std::result::Result<acp::LoadSessionResponse, acp::Error> {
        if !arguments.mcp_servers.is_empty() {
            StateBuilders::load_mcp_servers(self.services.as_ref(), &arguments.mcp_servers)
                .await
                .map_err(error::into_acp_error)?;
        }

        let session_key = arguments.session_id.0.as_ref().to_string();
        let conversation_id = ConversationId::parse(&session_key)
            .map_err(|error| acp::Error::into_internal_error(&error))?;

        let conversation = self
            .services
            .conversation_service()
            .find_conversation(&conversation_id)
            .await
            .map_err(|error| acp::Error::into_internal_error(&*error))?;
        if conversation.is_none() {
            return Err(acp::Error::invalid_params());
        }

        let active_agent_id = self
            .services
            .agent_registry()
            .get_active_agent_id()
            .await
            .map_err(|error| acp::Error::into_internal_error(&*error))?
            .unwrap_or_default();
        let state = self
            .ensure_session(&session_key, conversation_id, active_agent_id.clone())
            .await;

        let agent = self
            .services
            .agent_registry()
            .get_agent(&state.agent_id)
            .await
            .map_err(|error| acp::Error::into_internal_error(&*error))?
            .ok_or_else(|| acp::Error::invalid_params())?;

        let mode_state = StateBuilders::build_session_mode_state(
            self.services.as_ref(),
            &state.agent_id,
        )
        .await
        .map_err(error::into_acp_error)?;
        let model_state = StateBuilders::build_session_model_state(&self.services, &agent)
            .await
            .map_err(error::into_acp_error)?;

        Ok(acp::LoadSessionResponse::new()
            .modes(mode_state)
            .models(model_state))
    }

    pub(super) async fn handle_cancel(
        &self,
        arguments: acp::CancelNotification,
    ) -> std::result::Result<(), acp::Error> {
        let session_key = arguments.session_id.0.as_ref().to_string();
        let cancelled = self.cancel_session(&session_key).await;
        if !cancelled {
            tracing::warn!("No active ACP prompt to cancel for session {}", session_key);
        }
        Ok(())
    }

    pub(super) async fn handle_set_session_mode(
        &self,
        arguments: acp::SetSessionModeRequest,
    ) -> std::result::Result<acp::SetSessionModeResponse, acp::Error> {
        let session_key = arguments.session_id.0.as_ref().to_string();
        let mode_id = arguments.mode_id.0.as_ref();
        let agent_id = AgentId::new(mode_id);

        self.update_session_agent(&session_key, agent_id.clone())
            .await
            .map_err(error::into_acp_error)?;

        let notification = acp::SessionNotification::new(
            arguments.session_id,
            acp::SessionUpdate::CurrentModeUpdate(acp::CurrentModeUpdate::new(
                acp::SessionModeId::new(mode_id.to_string()),
            )),
        );
        self.send_notification(notification)
            .map_err(error::into_acp_error)?;

        Ok(acp::SetSessionModeResponse::new())
    }

    /// Handles session model changes.
    ///
    /// The model preference is stored per-session so that concurrent ACP
    /// clients do not interfere with each other. The global default model
    /// is also updated for backward compatibility with non-ACP code paths.
    pub(super) async fn handle_set_session_model(
        &self,
        arguments: acp::SetSessionModelRequest,
    ) -> std::result::Result<acp::SetSessionModelResponse, acp::Error> {
        let session_key = arguments.session_id.0.as_ref().to_string();
        let model_id = ModelId::new(arguments.model_id.0.to_string());

        // Store per-session model preference.
        self.update_session_model(&session_key, model_id.clone())
            .await
            .map_err(error::into_acp_error)?;

        // Also update the global default for backward compatibility.
        let provider_id = match self.services.get_session_config().await {
            Some(config) => config.provider,
            None => {
                let state = self
                    .session_state(&session_key)
                    .await
                    .map_err(error::into_acp_error)?;
                self.services
                    .agent_registry()
                    .get_agent(&state.agent_id)
                    .await
                    .map_err(|error| acp::Error::into_internal_error(&*error))?
                    .map(|agent| agent.provider)
                    .ok_or_else(acp::Error::invalid_params)?
            }
        };
        self.services
            .update_config(vec![ConfigOperation::SetSessionConfig(ModelConfig::new(
                provider_id,
                model_id.clone(),
            ))])
            .await
            .map_err(|error| acp::Error::into_internal_error(&*error))?;
        if let Err(error) = self.services.reload_agents().await {
            tracing::warn!("Failed to reload agents after model change: {}", error);
        }

        let notification = acp::SessionNotification::new(
            arguments.session_id,
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                acp::ContentBlock::Text(acp::TextContent::new(format!(
                    "Model changed to: {}\n\n",
                    model_id
                ))),
            )),
        );
        if let Err(error) = self.send_notification(notification) {
            tracing::warn!("Failed to send model change notification: {}", error);
        }

        Ok(acp::SetSessionModelResponse::default())
    }
}
