use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use forge_app::dto::ToolsOverview;
use forge_app::{
    AgentProviderResolver, AgentRegistry, AppConfigService, AuthService, CommandInfra,
    CommandLoaderService, ConversationService, DataGenerationApp, EnvironmentInfra,
    FileDiscoveryService, ForgeApp, GitApp, GrpcInfra, McpConfigManager, McpService,
    ProviderAuthService, ProviderService, Services, User, UserUsage, Walker, WorkspaceService,
};
use forge_config::ForgeConfig;
use forge_domain::{Agent, ConsoleWriter, *};
use forge_infra::ForgeInfra;
use forge_repo::ForgeRepo;
use forge_services::ForgeServices;
use forge_stream::MpscStream;
use futures::stream::BoxStream;
use url::Url;

use crate::API;

pub struct ForgeAPI<S, F> {
    services: Arc<S>,
    infra: Arc<F>,
}

impl<A, F> ForgeAPI<A, F> {
    pub fn new(services: Arc<A>, infra: Arc<F>) -> Self {
        Self { services, infra }
    }

    /// Creates a ForgeApp instance with the current services and latest config.
    fn app(&self) -> ForgeApp<A>
    where
        A: Services + EnvironmentInfra<Config = forge_config::ForgeConfig>,
        F: EnvironmentInfra<Config = forge_config::ForgeConfig>,
    {
        ForgeApp::new(self.services.clone())
    }
}

impl ForgeAPI<ForgeServices<ForgeRepo<ForgeInfra>>, ForgeRepo<ForgeInfra>> {
    /// Creates a fully-initialized [`ForgeAPI`] from a pre-read configuration.
    ///
    /// # Arguments
    /// * `cwd` - The working directory path for environment and file resolution
    /// * `config` - Pre-read application configuration (from startup)
    /// * `services_url` - Pre-validated URL for the gRPC workspace server
    pub fn init(cwd: PathBuf, config: ForgeConfig) -> Self {
        let infra = Arc::new(ForgeInfra::new(cwd, config));
        let repo = Arc::new(ForgeRepo::new(infra.clone()));
        let app = Arc::new(ForgeServices::new(repo.clone()));
        ForgeAPI::new(app, repo)
    }

    pub async fn get_skills_internal(&self) -> Result<Vec<Skill>> {
        use forge_domain::SkillRepository;
        self.infra.load_skills().await
    }
}

#[async_trait::async_trait]
impl<
    A: Services + EnvironmentInfra<Config = forge_config::ForgeConfig>,
    F: CommandInfra
        + EnvironmentInfra<Config = forge_config::ForgeConfig>
        + SkillRepository
        + GrpcInfra,
> API for ForgeAPI<A, F>
{
    async fn discover(&self) -> Result<Vec<File>> {
        let environment = self.services.get_environment();
        let config = Walker::unlimited().cwd(environment.cwd);
        self.services.collect_files(config).await
    }

    async fn get_tools(&self) -> anyhow::Result<ToolsOverview> {
        self.app().list_tools().await
    }

    async fn get_models(&self) -> Result<Vec<Model>> {
        self.app().get_models().await
    }

    async fn get_all_provider_models(&self) -> Result<Vec<ProviderModels>> {
        self.app().get_all_provider_models().await
    }

    async fn get_agents(&self) -> Result<Vec<Agent>> {
        self.services.get_agents().await
    }

    async fn get_agent_infos(&self) -> Result<Vec<AgentInfo>> {
        self.services.get_agent_infos().await
    }

    async fn get_providers(&self) -> Result<Vec<AnyProvider>> {
        Ok(self.services.get_all_providers().await?)
    }

    async fn commit(
        &self,
        preview: bool,
        max_diff_size: Option<usize>,
        diff: Option<String>,
        additional_context: Option<String>,
    ) -> Result<forge_app::CommitResult> {
        let git_app = GitApp::new(self.services.clone());
        let result = git_app
            .commit_message(max_diff_size, diff, additional_context)
            .await?;

        if preview {
            Ok(result)
        } else {
            git_app
                .commit(result.message, result.has_staged_files)
                .await
        }
    }

    async fn get_provider(&self, id: &ProviderId) -> Result<AnyProvider> {
        let providers = self.services.get_all_providers().await?;
        Ok(providers
            .into_iter()
            .find(|p| p.id() == *id)
            .ok_or_else(|| Error::provider_not_available(id.clone()))?)
    }

    async fn chat(
        &self,
        chat: ChatRequest,
    ) -> anyhow::Result<MpscStream<Result<ChatResponse, anyhow::Error>>> {
        let agent_id = self
            .services
            .get_active_agent_id()
            .await?
            .unwrap_or_default();
        self.app().chat(agent_id, chat).await
    }

    async fn upsert_conversation(&self, conversation: Conversation) -> anyhow::Result<()> {
        self.services.upsert_conversation(conversation).await
    }

    async fn compact_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> anyhow::Result<CompactionResult> {
        let agent_id = self
            .services
            .get_active_agent_id()
            .await?
            .unwrap_or_default();
        self.app()
            .compact_conversation(agent_id, conversation_id)
            .await
    }

    fn environment(&self) -> Environment {
        self.services.get_environment().clone()
    }

    async fn conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> anyhow::Result<Option<Conversation>> {
        self.services.find_conversation(conversation_id).await
    }

    async fn get_conversations(&self, limit: Option<usize>) -> anyhow::Result<Vec<Conversation>> {
        Ok(self
            .services
            .get_conversations(limit)
            .await?
            .unwrap_or_default())
    }

    async fn last_conversation(&self) -> anyhow::Result<Option<Conversation>> {
        self.services.last_conversation().await
    }

    async fn delete_conversation(&self, conversation_id: &ConversationId) -> anyhow::Result<()> {
        self.services.delete_conversation(conversation_id).await
    }

    async fn rename_conversation(
        &self,
        conversation_id: &ConversationId,
        title: String,
    ) -> anyhow::Result<()> {
        self.services
            .modify_conversation(conversation_id, |conv| {
                conv.title = Some(title);
            })
            .await
    }

    async fn execute_shell_command(
        &self,
        command: &str,
        working_dir: PathBuf,
    ) -> anyhow::Result<CommandOutput> {
        self.infra
            .execute_command(command.to_string(), working_dir, false, None)
            .await
    }
    async fn read_mcp_config(&self, scope: Option<&Scope>) -> Result<McpConfig> {
        self.services
            .read_mcp_config(scope)
            .await
            .map_err(|e| anyhow::anyhow!(e))
    }

    async fn write_mcp_config(&self, scope: &Scope, config: &McpConfig) -> Result<()> {
        self.services
            .write_mcp_config(config, scope)
            .await
            .map_err(|e| anyhow::anyhow!(e))
    }

    async fn execute_shell_command_raw(
        &self,
        command: &str,
    ) -> anyhow::Result<std::process::ExitStatus> {
        let cwd = self.environment().cwd;
        self.infra.execute_command_raw(command, cwd, None).await
    }

    async fn get_agent_provider(&self, agent_id: AgentId) -> anyhow::Result<Provider<Url>> {
        let agent_provider_resolver = AgentProviderResolver::new(self.services.clone());
        agent_provider_resolver.get_provider(Some(agent_id)).await
    }

    async fn update_config(&self, ops: Vec<forge_domain::ConfigOperation>) -> anyhow::Result<()> {
        // Determine whether any op affects provider/model resolution before writing,
        // so we can invalidate the agent cache afterwards.
        let needs_agent_reload = ops
            .iter()
            .any(|op| matches!(op, forge_domain::ConfigOperation::SetSessionConfig(_)));
        let result = self.services.update_config(ops).await;
        if needs_agent_reload {
            let _ = self.services.reload_agents().await;
        }
        result
    }

    async fn get_commit_config(&self) -> anyhow::Result<Option<ModelConfig>> {
        self.services.get_commit_config().await
    }

    async fn get_suggest_config(&self) -> anyhow::Result<Option<ModelConfig>> {
        self.services.get_suggest_config().await
    }

    async fn get_reasoning_effort(&self) -> anyhow::Result<Option<Effort>> {
        self.services.get_reasoning_effort().await
    }

    async fn user_info(&self) -> Result<Option<User>> {
        let provider = self.get_default_provider().await?;
        if let Some(api_key) = provider.api_key() {
            let user_info = self.services.user_info(api_key.as_str()).await?;
            return Ok(Some(user_info));
        }
        Ok(None)
    }

    async fn user_usage(&self) -> Result<Option<UserUsage>> {
        let provider = self.get_default_provider().await?;
        if let Some(api_key) = provider
            .credential
            .as_ref()
            .and_then(|c| match &c.auth_details {
                forge_domain::AuthDetails::ApiKey(key) => Some(key.as_str()),
                _ => None,
            })
        {
            let user_usage = self.services.user_usage(api_key).await?;
            return Ok(Some(user_usage));
        }
        Ok(None)
    }

    async fn get_active_agent(&self) -> Option<AgentId> {
        self.services.get_active_agent_id().await.ok().flatten()
    }

    async fn set_active_agent(&self, agent_id: AgentId) -> anyhow::Result<()> {
        self.services.set_active_agent_id(agent_id).await
    }

    async fn get_agent_model(&self, agent_id: AgentId) -> Option<ModelId> {
        let agent_provider_resolver = AgentProviderResolver::new(self.services.clone());
        agent_provider_resolver.get_model(Some(agent_id)).await.ok()
    }

    async fn get_default_model(&self) -> Option<ModelId> {
        self.services.get_provider_model(None).await.ok()
    }

    async fn reload_mcp(&self) -> Result<()> {
        self.services.mcp_service().reload_mcp().await
    }
    async fn get_commands(&self) -> Result<Vec<Command>> {
        self.services.get_commands().await
    }

    async fn get_skills(&self) -> Result<Vec<Skill>> {
        self.infra.load_skills().await
    }
    async fn generate_command(&self, prompt: UserPrompt) -> Result<String> {
        use forge_app::CommandGenerator;
        let generator = CommandGenerator::new(self.services.clone());
        generator.generate(prompt).await
    }

    async fn init_provider_auth(
        &self,
        provider_id: ProviderId,
        method: AuthMethod,
    ) -> Result<AuthContextRequest> {
        Ok(self
            .services
            .init_provider_auth(provider_id, method)
            .await?)
    }

    async fn complete_provider_auth(
        &self,
        provider_id: ProviderId,
        context: AuthContextResponse,
        timeout: Duration,
    ) -> Result<()> {
        Ok(self
            .services
            .complete_provider_auth(provider_id, context, timeout)
            .await?)
    }

    async fn remove_provider(&self, provider_id: &ProviderId) -> Result<()> {
        self.services.remove_credential(provider_id).await
    }

    async fn sync_workspace(
        &self,
        path: PathBuf,
    ) -> Result<MpscStream<Result<forge_domain::SyncProgress>>> {
        self.services.sync_workspace(path).await
    }

    async fn query_workspace(
        &self,
        path: PathBuf,
        params: forge_domain::SearchParams<'_>,
    ) -> Result<Vec<forge_domain::Node>> {
        self.services.query_workspace(path, params).await
    }

    async fn list_workspaces(&self) -> Result<Vec<forge_domain::WorkspaceInfo>> {
        self.services.list_workspaces().await
    }

    async fn get_workspace_info(
        &self,
        path: PathBuf,
    ) -> Result<Option<forge_domain::WorkspaceInfo>> {
        self.services.get_workspace_info(path).await
    }

    async fn delete_workspaces(&self, workspace_ids: Vec<forge_domain::WorkspaceId>) -> Result<()> {
        self.services.delete_workspaces(&workspace_ids).await
    }

    async fn get_workspace_status(&self, path: PathBuf) -> Result<Vec<forge_domain::FileStatus>> {
        self.services.get_workspace_status(path).await
    }

    async fn is_authenticated(&self) -> Result<bool> {
        self.services.is_authenticated().await
    }

    async fn create_auth_credentials(&self) -> Result<forge_domain::WorkspaceAuth> {
        self.services.init_auth_credentials().await
    }

    async fn init_workspace(&self, path: PathBuf) -> Result<forge_domain::WorkspaceId> {
        self.services.init_workspace(path).await
    }

    async fn migrate_env_credentials(&self) -> Result<Option<forge_domain::MigrationResult>> {
        Ok(self.services.migrate_env_credentials().await?)
    }

    async fn generate_data(
        &self,
        data_parameters: DataGenerationParameters,
    ) -> Result<BoxStream<'static, Result<serde_json::Value, anyhow::Error>>> {
        let app = DataGenerationApp::new(self.services.clone());
        app.execute(data_parameters).await
    }

    async fn get_default_provider(&self) -> Result<Provider<Url>> {
        let provider_id = self.services.get_default_provider().await?;
        self.services.get_provider(provider_id).await
    }

    async fn mcp_auth(&self, server_url: &str) -> Result<()> {
        let env = self.services.get_environment().clone();
        forge_infra::mcp_auth(server_url, &env).await
    }

    async fn mcp_logout(&self, server_url: Option<&str>) -> Result<()> {
        let env = self.services.get_environment().clone();
        match server_url {
            Some(url) => forge_infra::mcp_logout(url, &env).await,
            None => forge_infra::mcp_logout_all(&env).await,
        }
    }

    async fn mcp_auth_status(&self, server_url: &str) -> Result<String> {
        let env = self.services.get_environment().clone();
        Ok(forge_infra::mcp_auth_status(server_url, &env).await)
    }

    fn hydrate_channel(&self) -> Result<()> {
        self.infra.hydrate();
        Ok(())
    }
}

impl<A: Send + Sync, F: ConsoleWriter> ConsoleWriter for ForgeAPI<A, F> {
    fn write(&self, buf: &[u8]) -> std::io::Result<usize> {
        self.infra.write(buf)
    }

    fn write_err(&self, buf: &[u8]) -> std::io::Result<usize> {
        self.infra.write_err(buf)
    }

    fn flush(&self) -> std::io::Result<()> {
        self.infra.flush()
    }

    fn flush_err(&self) -> std::io::Result<()> {
        self.infra.flush_err()
    }
}
