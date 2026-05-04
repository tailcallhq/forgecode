use agent_client_protocol as acp;
use forge_config::ForgeConfig;
use forge_domain::{AgentId, ConfigOperation, Conversation, ConversationId, ModelConfig, ModelId};

use crate::{AgentRegistry, AppConfigService, ConversationService, EnvironmentInfra, Services};

use super::adapter::{AcpAdapter, SessionState};
use super::error;
use super::state_builders::StateBuilders;

const VERSION: &str = env!("CARGO_PKG_VERSION");

impl<S> AcpAdapter<S> {
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
}

impl<S: Services + EnvironmentInfra<Config = ForgeConfig>> AcpAdapter<S> {
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

impl<S> AcpAdapter<S> {
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
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use agent_client_protocol as acp;
    use forge_config::ForgeConfig;
    use forge_domain::{
        Agent, AgentId, AnyProvider, Attachment, AuthContextRequest, AuthContextResponse,
        AuthMethod, ChatCompletionMessage, CommandOutput, ConfigOperation, Context, Conversation,
        ConversationId, File, FileStatus, Image, McpConfig, McpServers, Model, ModelConfig,
        ModelId, Node, Provider, ProviderId, ProviderResponse, ProviderType, Scope, SearchParams,
        Skill, SyncProgress, Template, ToolCallFull, ToolOutput, URLParamSpec, WorkspaceAuth,
        WorkspaceId, WorkspaceInfo,
    };
    use reqwest::Url;

    use super::{AcpAdapter, SessionState};
    use crate::infra::EnvironmentInfra;
    use crate::services::{
        AgentRegistry, AppConfigService, AttachmentService, AuthService, CommandLoaderService,
        ConversationService, CustomInstructionsService, FileDiscoveryService, FollowUpService,
        FsPatchService, FsReadService, FsRemoveService, FsSearchService, FsUndoService,
        FsWriteService, HttpResponse, ImageReadService, McpConfigManager, McpService,
        NetFetchService, PatchOutput, PlanCreateOutput, PlanCreateService, PolicyDecision,
        PolicyService, ProviderAuthService, ProviderService, ReadOutput, ResponseContext,
        SearchResult, Services, ShellOutput, ShellService, SkillFetchService, TemplateService,
        WorkspaceService,
    };
    use crate::user::{AuthProviderId, Plan, UsageInfo, User, UserUsage};
    use crate::Walker;

    #[derive(Clone)]
    struct SharedState(Arc<Mutex<MockState>>);

    #[derive(Clone)]
    struct MockServices {
        provider_service: MockProviderService,
        config_service: MockConfigService,
        conversation_service: MockConversationService,
        mcp_config_manager: MockMcpConfigService,
        agent_registry: MockAgentRegistryService,
        noop_service: NoopService,
        environment: forge_domain::Environment,
        config: ForgeConfig,
    }

    struct MockState {
        active_agent_id: Option<AgentId>,
        agents: Vec<Agent>,
        conversations: HashMap<ConversationId, Conversation>,
        provider: Provider<Url>,
        models: Vec<Model>,
        mcp_config: McpConfig,
        config_updates: Vec<Vec<ConfigOperation>>,
    }

    #[derive(Clone)]
    struct MockProviderService {
        state: SharedState,
    }

    #[derive(Clone)]
    struct MockConfigService {
        state: SharedState,
    }

    #[derive(Clone)]
    struct MockConversationService {
        state: SharedState,
    }

    #[derive(Clone)]
    struct MockMcpConfigService {
        state: SharedState,
    }

    #[derive(Clone)]
    struct MockAgentRegistryService {
        state: SharedState,
    }

    #[derive(Clone, Default)]
    struct NoopService;

    impl MockServices {
        fn new() -> Self {
            let agent = Agent::new(
                AgentId::new("forge"),
                ProviderId::OPENAI,
                ModelId::new("test-model"),
            )
            .title("Forge")
            .description("Test agent");
            let provider = Provider {
                id: ProviderId::OPENAI,
                provider_type: ProviderType::Llm,
                response: Some(ProviderResponse::OpenAI),
                url: Url::parse("https://api.example.com/chat").unwrap(),
                models: None,
                auth_methods: vec![AuthMethod::ApiKey],
                url_params: Vec::<URLParamSpec>::new(),
                credential: None,
                custom_headers: None,
            };
            let model = Model {
                id: ModelId::new("test-model"),
                name: Some("Test Model".to_string()),
                description: Some("Model used by ACP tests".to_string()),
                context_length: Some(8192),
                tools_supported: Some(true),
                supports_parallel_tool_calls: Some(true),
                supports_reasoning: Some(false),
                input_modalities: vec![forge_domain::InputModality::Text],
            };
            let state = SharedState(Arc::new(Mutex::new(MockState {
                active_agent_id: Some(agent.id.clone()),
                agents: vec![agent],
                conversations: HashMap::new(),
                provider: provider.clone(),
                models: vec![model],
                mcp_config: McpConfig::default(),
                config_updates: Vec::new(),
            })));

            Self {
                provider_service: MockProviderService {
                    state: state.clone(),
                },
                config_service: MockConfigService {
                    state: state.clone(),
                },
                conversation_service: MockConversationService {
                    state: state.clone(),
                },
                mcp_config_manager: MockMcpConfigService {
                    state: state.clone(),
                },
                agent_registry: MockAgentRegistryService { state },
                noop_service: NoopService,
                environment: forge_domain::Environment {
                    os: "macos".to_string(),
                    cwd: PathBuf::from("/tmp/project"),
                    home: Some(PathBuf::from("/tmp/home")),
                    shell: "/bin/zsh".to_string(),
                    base_path: PathBuf::from("/tmp/forge"),
                },
                config: ForgeConfig::default(),
            }
        }

        fn insert_conversation(&self, conversation: Conversation) {
            self.conversation_service
                .state
                .0
                .lock()
                .unwrap()
                .conversations
                .insert(conversation.id, conversation);
        }

        fn config_updates(&self) -> Vec<Vec<ConfigOperation>> {
            self.config_service
                .state
                .0
                .lock()
                .unwrap()
                .config_updates
                .clone()
        }
    }

    impl EnvironmentInfra for MockServices {
        type Config = ForgeConfig;

        fn get_env_var(&self, _key: &str) -> Option<String> {
            None
        }

        fn get_env_vars(&self) -> BTreeMap<String, String> {
            BTreeMap::new()
        }

        fn get_environment(&self) -> forge_domain::Environment {
            self.environment.clone()
        }

        fn get_config(&self) -> anyhow::Result<Self::Config> {
            Ok(self.config.clone())
        }

        fn update_environment(
            &self,
            _ops: Vec<ConfigOperation>,
        ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send {
            async { Ok(()) }
        }
    }

    #[async_trait::async_trait]
    impl ProviderService for MockProviderService {
        async fn chat(
            &self,
            _model_id: &ModelId,
            _context: Context,
            _provider: Provider<Url>,
        ) -> forge_domain::ResultStream<ChatCompletionMessage, anyhow::Error> {
            todo!("unused in session handler tests")
        }

        async fn models(&self, _provider: Provider<Url>) -> anyhow::Result<Vec<Model>> {
            Ok(self.state.0.lock().unwrap().models.clone())
        }

        async fn get_provider(&self, _id: ProviderId) -> anyhow::Result<Provider<Url>> {
            Ok(self.state.0.lock().unwrap().provider.clone())
        }

        async fn get_all_providers(&self) -> anyhow::Result<Vec<AnyProvider>> {
            Ok(vec![AnyProvider::Url(self.state.0.lock().unwrap().provider.clone())])
        }

        async fn upsert_credential(
            &self,
            _credential: forge_domain::AuthCredential,
        ) -> anyhow::Result<()> {
            todo!("unused in session handler tests")
        }

        async fn remove_credential(&self, _id: &ProviderId) -> anyhow::Result<()> {
            todo!("unused in session handler tests")
        }

        async fn migrate_env_credentials(
            &self,
        ) -> anyhow::Result<Option<forge_domain::MigrationResult>> {
            Ok(None)
        }
    }

    #[async_trait::async_trait]
    impl AppConfigService for MockConfigService {
        async fn get_session_config(&self) -> Option<ModelConfig> {
            Some(ModelConfig::new(ProviderId::OPENAI, ModelId::new("test-model")))
        }

        async fn get_commit_config(&self) -> anyhow::Result<Option<ModelConfig>> {
            Ok(None)
        }

        async fn get_suggest_config(&self) -> anyhow::Result<Option<ModelConfig>> {
            Ok(None)
        }

        async fn get_reasoning_effort(&self) -> anyhow::Result<Option<forge_domain::Effort>> {
            Ok(None)
        }

        async fn update_config(&self, ops: Vec<ConfigOperation>) -> anyhow::Result<()> {
            self.state.0.lock().unwrap().config_updates.push(ops);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl ConversationService for MockConversationService {
        async fn find_conversation(&self, id: &ConversationId) -> anyhow::Result<Option<Conversation>> {
            Ok(self.state.0.lock().unwrap().conversations.get(id).cloned())
        }

        async fn upsert_conversation(&self, conversation: Conversation) -> anyhow::Result<()> {
            self.state
                .0
                .lock()
                .unwrap()
                .conversations
                .insert(conversation.id, conversation);
            Ok(())
        }

        async fn modify_conversation<F, T>(&self, id: &ConversationId, f: F) -> anyhow::Result<T>
        where
            F: FnOnce(&mut Conversation) -> T + Send,
            T: Send,
        {
            let mut guard = self.state.0.lock().unwrap();
            let conversation = guard.conversations.get_mut(id).expect("conversation must exist");
            Ok(f(conversation))
        }

        async fn get_conversations(
            &self,
            _limit: Option<usize>,
        ) -> anyhow::Result<Option<Vec<Conversation>>> {
            Ok(Some(
                self.state
                    .0
                    .lock()
                    .unwrap()
                    .conversations
                    .values()
                    .cloned()
                    .collect(),
            ))
        }

        async fn last_conversation(&self) -> anyhow::Result<Option<Conversation>> {
            Ok(self
                .state
                .0
                .lock()
                .unwrap()
                .conversations
                .values()
                .last()
                .cloned())
        }

        async fn delete_conversation(&self, conversation_id: &ConversationId) -> anyhow::Result<()> {
            self.state.0.lock().unwrap().conversations.remove(conversation_id);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl AgentRegistry for MockAgentRegistryService {
        async fn get_active_agent_id(&self) -> anyhow::Result<Option<AgentId>> {
            Ok(self.state.0.lock().unwrap().active_agent_id.clone())
        }

        async fn set_active_agent_id(&self, agent_id: AgentId) -> anyhow::Result<()> {
            self.state.0.lock().unwrap().active_agent_id = Some(agent_id);
            Ok(())
        }

        async fn get_agents(&self) -> anyhow::Result<Vec<Agent>> {
            Ok(self.state.0.lock().unwrap().agents.clone())
        }

        async fn get_agent_infos(&self) -> anyhow::Result<Vec<forge_domain::AgentInfo>> {
            Ok(self
                .state
                .0
                .lock()
                .unwrap()
                .agents
                .iter()
                .map(|agent| {
                    let mut info = forge_domain::AgentInfo::default().id(agent.id.clone());
                    if let Some(title) = agent.title.clone() {
                        info = info.title(title);
                    }
                    if let Some(description) = agent.description.clone() {
                        info = info.description(description);
                    }
                    info
                })
                .collect())
        }

        async fn get_agent(&self, agent_id: &AgentId) -> anyhow::Result<Option<Agent>> {
            Ok(self
                .state
                .0
                .lock()
                .unwrap()
                .agents
                .iter()
                .find(|agent| &agent.id == agent_id)
                .cloned())
        }

        async fn reload_agents(&self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl McpConfigManager for MockMcpConfigService {
        async fn read_mcp_config(&self, _scope: Option<&Scope>) -> anyhow::Result<McpConfig> {
            Ok(self.state.0.lock().unwrap().mcp_config.clone())
        }

        async fn write_mcp_config(&self, config: &McpConfig, _scope: &Scope) -> anyhow::Result<()> {
            self.state.0.lock().unwrap().mcp_config = config.clone();
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl McpService for NoopService {
        async fn get_mcp_servers(&self) -> anyhow::Result<McpServers> {
            Ok(McpServers::default())
        }

        async fn execute_mcp(&self, _call: ToolCallFull) -> anyhow::Result<ToolOutput> {
            todo!("unused in session handler tests")
        }

        async fn reload_mcp(&self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl ProviderAuthService for NoopService {
        async fn init_provider_auth(
            &self,
            _provider_id: ProviderId,
            _method: AuthMethod,
        ) -> anyhow::Result<AuthContextRequest> {
            todo!("unused in session handler tests")
        }

        async fn complete_provider_auth(
            &self,
            _provider_id: ProviderId,
            _context: AuthContextResponse,
            _timeout: std::time::Duration,
        ) -> anyhow::Result<()> {
            todo!("unused in session handler tests")
        }

        async fn refresh_provider_credential(
            &self,
            provider: Provider<Url>,
        ) -> anyhow::Result<Provider<Url>> {
            Ok(provider)
        }
    }

    #[async_trait::async_trait]
    impl TemplateService for NoopService {
        async fn register_template(&self, _path: PathBuf) -> anyhow::Result<()> {
            todo!("unused in session handler tests")
        }

        async fn render_template<V: serde::Serialize + Send + Sync>(
            &self,
            _template: Template<V>,
            _object: &V,
        ) -> anyhow::Result<String> {
            todo!("unused in session handler tests")
        }
    }

    #[async_trait::async_trait]
    impl AttachmentService for NoopService {
        async fn attachments(&self, _url: &str) -> anyhow::Result<Vec<Attachment>> {
            Ok(vec![])
        }
    }

    #[async_trait::async_trait]
    impl CustomInstructionsService for NoopService {
        async fn get_custom_instructions(&self) -> Vec<String> {
            vec![]
        }
    }

    #[async_trait::async_trait]
    impl FileDiscoveryService for NoopService {
        async fn collect_files(&self, _config: Walker) -> anyhow::Result<Vec<File>> {
            Ok(vec![])
        }

        async fn list_current_directory(&self) -> anyhow::Result<Vec<File>> {
            Ok(vec![])
        }
    }

    #[async_trait::async_trait]
    impl FsWriteService for NoopService {
        async fn write(
            &self,
            _path: String,
            _content: String,
            _overwrite: bool,
        ) -> anyhow::Result<crate::services::FsWriteOutput> {
            todo!("unused in session handler tests")
        }
    }

    #[async_trait::async_trait]
    impl PlanCreateService for NoopService {
        async fn create_plan(
            &self,
            _plan_name: String,
            _version: String,
            _content: String,
        ) -> anyhow::Result<PlanCreateOutput> {
            todo!("unused in session handler tests")
        }
    }

    #[async_trait::async_trait]
    impl FsPatchService for NoopService {
        async fn patch(
            &self,
            _path: String,
            _search: String,
            _content: String,
            _replace_all: bool,
        ) -> anyhow::Result<PatchOutput> {
            todo!("unused in session handler tests")
        }

        async fn multi_patch(
            &self,
            _path: String,
            _edits: Vec<forge_domain::PatchEdit>,
        ) -> anyhow::Result<PatchOutput> {
            todo!("unused in session handler tests")
        }
    }

    #[async_trait::async_trait]
    impl FsReadService for NoopService {
        async fn read(
            &self,
            _path: String,
            _start_line: Option<u64>,
            _end_line: Option<u64>,
        ) -> anyhow::Result<ReadOutput> {
            todo!("unused in session handler tests")
        }
    }

    #[async_trait::async_trait]
    impl ImageReadService for NoopService {
        async fn read_image(&self, _path: String) -> anyhow::Result<Image> {
            todo!("unused in session handler tests")
        }
    }

    #[async_trait::async_trait]
    impl FsRemoveService for NoopService {
        async fn remove(&self, _path: String) -> anyhow::Result<crate::services::FsRemoveOutput> {
            todo!("unused in session handler tests")
        }
    }

    #[async_trait::async_trait]
    impl FsSearchService for NoopService {
        async fn search(&self, _params: forge_domain::FSSearch) -> anyhow::Result<Option<SearchResult>> {
            Ok(None)
        }
    }

    #[async_trait::async_trait]
    impl FollowUpService for NoopService {
        async fn follow_up(
            &self,
            _question: String,
            _options: Vec<String>,
            _multiple: Option<bool>,
        ) -> anyhow::Result<Option<String>> {
            Ok(None)
        }
    }

    #[async_trait::async_trait]
    impl FsUndoService for NoopService {
        async fn undo(&self, _path: String) -> anyhow::Result<crate::services::FsUndoOutput> {
            todo!("unused in session handler tests")
        }
    }

    #[async_trait::async_trait]
    impl NetFetchService for NoopService {
        async fn fetch(&self, _url: String, _raw: Option<bool>) -> anyhow::Result<HttpResponse> {
            Ok(HttpResponse {
                content: String::new(),
                code: 200,
                context: ResponseContext::Raw,
                content_type: "text/plain".to_string(),
            })
        }
    }

    #[async_trait::async_trait]
    impl ShellService for NoopService {
        async fn execute(
            &self,
            _command: String,
            _cwd: PathBuf,
            _keep_ansi: bool,
            _silent: bool,
            _env_vars: Option<Vec<String>>,
            _description: Option<String>,
        ) -> anyhow::Result<ShellOutput> {
            Ok(ShellOutput {
                output: CommandOutput {
                    command: String::new(),
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: Some(0),
                },
                shell: "/bin/zsh".to_string(),
                description: None,
            })
        }
    }

    #[async_trait::async_trait]
    impl AuthService for NoopService {
        async fn user_info(&self, _api_key: &str) -> anyhow::Result<User> {
            Ok(User {
                auth_provider_id: AuthProviderId::new("test"),
            })
        }

        async fn user_usage(&self, _api_key: &str) -> anyhow::Result<UserUsage> {
            Ok(UserUsage {
                plan: Plan { r#type: "free".to_string() },
                usage: UsageInfo {
                    current: 0,
                    limit: 0,
                    remaining: 0,
                    reset_in: None,
                },
            })
        }
    }

    #[async_trait::async_trait]
    impl CommandLoaderService for NoopService {
        async fn get_commands(&self) -> anyhow::Result<Vec<forge_domain::Command>> {
            Ok(vec![])
        }
    }

    #[async_trait::async_trait]
    impl PolicyService for NoopService {
        async fn check_operation_permission(
            &self,
            _operation: &forge_domain::PermissionOperation,
        ) -> anyhow::Result<PolicyDecision> {
            Ok(PolicyDecision {
                allowed: true,
                path: None,
            })
        }
    }

    #[async_trait::async_trait]
    impl WorkspaceService for NoopService {
        async fn sync_workspace(
            &self,
            _path: PathBuf,
        ) -> anyhow::Result<forge_stream::MpscStream<anyhow::Result<SyncProgress>>> {
            todo!("unused in session handler tests")
        }

        async fn query_workspace(
            &self,
            _path: PathBuf,
            _params: SearchParams<'_>,
        ) -> anyhow::Result<Vec<Node>> {
            todo!("unused in session handler tests")
        }

        async fn list_workspaces(&self) -> anyhow::Result<Vec<WorkspaceInfo>> {
            Ok(vec![])
        }

        async fn get_workspace_info(&self, _path: PathBuf) -> anyhow::Result<Option<WorkspaceInfo>> {
            Ok(None)
        }

        async fn delete_workspace(&self, _workspace_id: &WorkspaceId) -> anyhow::Result<()> {
            Ok(())
        }

        async fn delete_workspaces(&self, _workspace_ids: &[WorkspaceId]) -> anyhow::Result<()> {
            Ok(())
        }

        async fn is_indexed(&self, _path: &Path) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn get_workspace_status(&self, _path: PathBuf) -> anyhow::Result<Vec<FileStatus>> {
            Ok(vec![])
        }

        async fn is_authenticated(&self) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn init_auth_credentials(&self) -> anyhow::Result<WorkspaceAuth> {
            todo!("unused in session handler tests")
        }

        async fn init_workspace(&self, _path: PathBuf) -> anyhow::Result<WorkspaceId> {
            todo!("unused in session handler tests")
        }
    }

    #[async_trait::async_trait]
    impl SkillFetchService for NoopService {
        async fn fetch_skill(&self, _skill_name: String) -> anyhow::Result<Skill> {
            todo!("unused in session handler tests")
        }

        async fn list_skills(&self) -> anyhow::Result<Vec<Skill>> {
            Ok(vec![])
        }
    }

    impl Services for MockServices {
        type ProviderService = MockProviderService;
        type AppConfigService = MockConfigService;
        type ConversationService = MockConversationService;
        type TemplateService = NoopService;
        type AttachmentService = NoopService;
        type CustomInstructionsService = NoopService;
        type FileDiscoveryService = NoopService;
        type McpConfigManager = MockMcpConfigService;
        type FsWriteService = NoopService;
        type PlanCreateService = NoopService;
        type FsPatchService = NoopService;
        type FsReadService = NoopService;
        type ImageReadService = NoopService;
        type FsRemoveService = NoopService;
        type FsSearchService = NoopService;
        type FollowUpService = NoopService;
        type FsUndoService = NoopService;
        type NetFetchService = NoopService;
        type ShellService = NoopService;
        type McpService = NoopService;
        type AuthService = NoopService;
        type AgentRegistry = MockAgentRegistryService;
        type CommandLoaderService = NoopService;
        type PolicyService = NoopService;
        type ProviderAuthService = NoopService;
        type WorkspaceService = NoopService;
        type SkillFetchService = NoopService;

        fn provider_service(&self) -> &Self::ProviderService { &self.provider_service }
        fn config_service(&self) -> &Self::AppConfigService { &self.config_service }
        fn conversation_service(&self) -> &Self::ConversationService { &self.conversation_service }
        fn template_service(&self) -> &Self::TemplateService { &self.noop_service }
        fn attachment_service(&self) -> &Self::AttachmentService { &self.noop_service }
        fn file_discovery_service(&self) -> &Self::FileDiscoveryService { &self.noop_service }
        fn mcp_config_manager(&self) -> &Self::McpConfigManager { &self.mcp_config_manager }
        fn fs_create_service(&self) -> &Self::FsWriteService { &self.noop_service }
        fn plan_create_service(&self) -> &Self::PlanCreateService { &self.noop_service }
        fn fs_patch_service(&self) -> &Self::FsPatchService { &self.noop_service }
        fn fs_read_service(&self) -> &Self::FsReadService { &self.noop_service }
        fn image_read_service(&self) -> &Self::ImageReadService { &self.noop_service }
        fn fs_remove_service(&self) -> &Self::FsRemoveService { &self.noop_service }
        fn fs_search_service(&self) -> &Self::FsSearchService { &self.noop_service }
        fn follow_up_service(&self) -> &Self::FollowUpService { &self.noop_service }
        fn fs_undo_service(&self) -> &Self::FsUndoService { &self.noop_service }
        fn net_fetch_service(&self) -> &Self::NetFetchService { &self.noop_service }
        fn shell_service(&self) -> &Self::ShellService { &self.noop_service }
        fn mcp_service(&self) -> &Self::McpService { &self.noop_service }
        fn custom_instructions_service(&self) -> &Self::CustomInstructionsService { &self.noop_service }
        fn auth_service(&self) -> &Self::AuthService { &self.noop_service }
        fn agent_registry(&self) -> &Self::AgentRegistry { &self.agent_registry }
        fn command_loader_service(&self) -> &Self::CommandLoaderService { &self.noop_service }
        fn policy_service(&self) -> &Self::PolicyService { &self.noop_service }
        fn provider_auth_service(&self) -> &Self::ProviderAuthService { &self.noop_service }
        fn workspace_service(&self) -> &Self::WorkspaceService { &self.noop_service }
        fn skill_fetch_service(&self) -> &Self::SkillFetchService { &self.noop_service }
    }

    #[tokio::test]
    async fn initialize_exposes_acp_capabilities() {
        let adapter = AcpAdapter::new_for_test(());

        let actual = adapter
            .handle_initialize(acp::InitializeRequest::new(acp::ProtocolVersion::V1))
            .await
            .unwrap();

        assert_eq!(actual.protocol_version, acp::ProtocolVersion::V1);
        assert!(actual.agent_capabilities.load_session);
        assert!(actual.agent_capabilities.mcp_capabilities.http);
        assert!(actual.agent_capabilities.mcp_capabilities.sse);
    }

    #[tokio::test]
    async fn authenticate_is_a_no_op() {
        let adapter = AcpAdapter::new_for_test(());

        let actual = adapter
            .handle_authenticate(acp::AuthenticateRequest::new(acp::AuthMethodId::new("stdio")))
            .await
            .unwrap();

        assert_eq!(actual, acp::AuthenticateResponse::default());
    }

    #[tokio::test]
    async fn cancel_returns_ok_when_session_is_missing() {
        let adapter = AcpAdapter::new_for_test(());

        let actual = adapter
            .handle_cancel(acp::CancelNotification::new(acp::SessionId::new("missing")))
            .await;

        assert!(actual.is_ok());
    }

    #[tokio::test]
    async fn set_session_mode_updates_state_and_emits_notification() {
        let (adapter, mut rx) = AcpAdapter::new_for_test_with_receiver(());
        let session_id = acp::SessionId::new("session-4");
        let conversation_id = ConversationId::generate();

        adapter
            .store_session(
                "session-4".to_string(),
                SessionState {
                    conversation_id,
                    agent_id: AgentId::new("before"),
                    model_id: None,
                    cancel_notify: None,
                },
            )
            .await;

        let actual = adapter
            .handle_set_session_mode(acp::SetSessionModeRequest::new(
                session_id.clone(),
                acp::SessionModeId::new("after"),
            ))
            .await;

        assert!(actual.is_ok());
        let state = adapter.session_state("session-4").await.unwrap();
        assert_eq!(state.agent_id, AgentId::new("after"));

        let notification = rx.recv().await;
        assert!(notification.is_some());
        let notification = notification.unwrap();
        assert_eq!(notification.session_id, session_id);
    }

    #[tokio::test]
    async fn new_session_creates_conversation_and_returns_initial_state() {
        let services = MockServices::new();
        let adapter = AcpAdapter::new_for_test(services.clone());

        let actual = adapter
            .handle_new_session(acp::NewSessionRequest::new(PathBuf::from("/tmp/project")))
            .await
            .unwrap();

        let conversation_id = ConversationId::parse(actual.session_id.0.as_ref()).unwrap();
        let stored = services.find_conversation(&conversation_id).await.unwrap();

        assert!(stored.is_some());
        assert_eq!(
            actual
                .modes
                .as_ref()
                .map(|modes| modes.current_mode_id.0.as_ref()),
            Some("forge")
        );
        assert_eq!(
            actual
                .models
                .as_ref()
                .map(|models| models.current_model_id.0.as_ref()),
            Some("test-model")
        );
        let session = adapter.session_state(actual.session_id.0.as_ref()).await.unwrap();
        assert_eq!(session.agent_id, AgentId::new("forge"));
    }

    #[tokio::test]
    async fn load_session_returns_invalid_params_for_unknown_conversation() {
        let services = MockServices::new();
        let adapter = AcpAdapter::new_for_test(services);

        let actual = adapter
            .handle_load_session(acp::LoadSessionRequest::new(
                acp::SessionId::new(ConversationId::generate().into_string()),
                PathBuf::from("/tmp/project"),
            ))
            .await;

        assert!(actual.is_err());
        assert_eq!(actual.unwrap_err().code, acp::ErrorCode::InvalidParams);
    }

    #[tokio::test]
    async fn load_session_uses_existing_conversation_and_builds_state() {
        let services = MockServices::new();
        let adapter = AcpAdapter::new_for_test(services.clone());
        let conversation = Conversation::generate();
        let conversation_id = conversation.id;
        services.insert_conversation(conversation);

        let actual = adapter
            .handle_load_session(acp::LoadSessionRequest::new(
                acp::SessionId::new(conversation_id.into_string()),
                PathBuf::from("/tmp/project"),
            ))
            .await
            .unwrap();

        assert_eq!(
            actual
                .modes
                .as_ref()
                .map(|modes| modes.current_mode_id.0.as_ref()),
            Some("forge")
        );
        assert_eq!(
            actual
                .models
                .as_ref()
                .map(|models| models.current_model_id.0.as_ref()),
            Some("test-model")
        );
        let session = adapter
            .session_state(conversation_id.into_string().as_str())
            .await
            .unwrap();
        assert_eq!(session.conversation_id, conversation_id);
    }

    #[tokio::test]
    async fn set_session_model_updates_session_and_config() {
        let (adapter, mut rx) = AcpAdapter::new_for_test_with_receiver(MockServices::new());
        let conversation = Conversation::generate();
        let session_id = acp::SessionId::new(conversation.id.into_string());

        adapter
            .store_session(
                session_id.0.as_ref().to_string(),
                SessionState {
                    conversation_id: conversation.id,
                    agent_id: AgentId::new("forge"),
                    model_id: None,
                    cancel_notify: None,
                },
            )
            .await;

        let actual = adapter
            .handle_set_session_model(acp::SetSessionModelRequest::new(
                session_id.clone(),
                acp::ModelId::new("gpt-test"),
            ))
            .await;

        assert!(actual.is_ok());
        let session = adapter.session_state(session_id.0.as_ref()).await.unwrap();
        assert_eq!(session.model_id, Some(ModelId::new("gpt-test")));

        let updates = adapter.services.config_updates();
        assert_eq!(
            updates,
            vec![vec![ConfigOperation::SetSessionConfig(ModelConfig::new(
                ProviderId::OPENAI,
                ModelId::new("gpt-test"),
            ))]]
        );

        let notification = rx.recv().await.expect("expected model change notification");
        assert_eq!(notification.session_id, session_id);
    }
}
