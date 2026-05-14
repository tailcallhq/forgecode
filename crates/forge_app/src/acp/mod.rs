mod adapter;
mod conversion;
mod error;
mod prompt_handler;
mod session_handlers;
mod state_builders;

pub(crate) use adapter::AcpAdapter;

#[async_trait::async_trait(?Send)]
impl<S: crate::Services + crate::EnvironmentInfra<Config = forge_config::ForgeConfig>>
    agent_client_protocol::Agent for AcpAdapter<S>
{
    async fn initialize(
        &self,
        arguments: agent_client_protocol::InitializeRequest,
    ) -> std::result::Result<
        agent_client_protocol::InitializeResponse,
        agent_client_protocol::Error,
    > {
        self.handle_initialize(arguments).await
    }

    async fn authenticate(
        &self,
        arguments: agent_client_protocol::AuthenticateRequest,
    ) -> std::result::Result<
        agent_client_protocol::AuthenticateResponse,
        agent_client_protocol::Error,
    > {
        self.handle_authenticate(arguments).await
    }

    async fn new_session(
        &self,
        arguments: agent_client_protocol::NewSessionRequest,
    ) -> std::result::Result<
        agent_client_protocol::NewSessionResponse,
        agent_client_protocol::Error,
    > {
        self.handle_new_session(arguments).await
    }

    async fn load_session(
        &self,
        arguments: agent_client_protocol::LoadSessionRequest,
    ) -> std::result::Result<
        agent_client_protocol::LoadSessionResponse,
        agent_client_protocol::Error,
    > {
        self.handle_load_session(arguments).await
    }

    async fn prompt(
        &self,
        arguments: agent_client_protocol::PromptRequest,
    ) -> std::result::Result<
        agent_client_protocol::PromptResponse,
        agent_client_protocol::Error,
    > {
        self.handle_prompt(arguments).await
    }

    async fn cancel(
        &self,
        arguments: agent_client_protocol::CancelNotification,
    ) -> std::result::Result<(), agent_client_protocol::Error> {
        self.handle_cancel(arguments).await
    }

    async fn set_session_mode(
        &self,
        arguments: agent_client_protocol::SetSessionModeRequest,
    ) -> std::result::Result<
        agent_client_protocol::SetSessionModeResponse,
        agent_client_protocol::Error,
    > {
        self.handle_set_session_mode(arguments).await
    }

    async fn set_session_model(
        &self,
        arguments: agent_client_protocol::SetSessionModelRequest,
    ) -> std::result::Result<
        agent_client_protocol::SetSessionModelResponse,
        agent_client_protocol::Error,
    > {
        self.handle_set_session_model(arguments).await
    }
}
