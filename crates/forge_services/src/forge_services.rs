use std::sync::Arc;

use forge_app::{
    AgentRepository, CommandInfra, DirectoryReaderInfra, EnvironmentInfra, FileDirectoryInfra,
    FileInfoInfra, FileReaderInfra, FileRemoverInfra, FileWriterInfra, HttpInfra, KVStore,
    McpServerInfra, Services, StrategyFactory, UserInfra, WalkerInfra,
};
use forge_domain::{
    ChatRepository, ConversationRepository, FuzzySearchRepository, ProviderRepository,
    SkillRepository, SnapshotRepository, ValidationRepository, WorkspaceIndexRepository,
};

use crate::ForgeProviderAuthService;
use crate::agent_registry::ForgeAgentRegistryService;
use crate::app_config::ForgeAppConfigService;
use crate::attachment::ForgeChatRequest;
use crate::auth::ForgeAuthService;
use crate::command::CommandLoaderService as ForgeCommandLoaderService;
use crate::conversation::ForgeConversationService;
use crate::discovery::ForgeDiscoveryService;
use crate::fd::FdDefault;
use crate::instructions::ForgeCustomInstructionsService;
use crate::mcp::{ForgeMcpManager, ForgeMcpService};
use crate::policy::ForgePolicyService;
use crate::provider_service::ForgeProviderService;
use crate::template::ForgeTemplateService;
use crate::tool_services::{
    ForgeFetch, ForgeFollowup, ForgeFsPatch, ForgeFsRead, ForgeFsRemove, ForgeFsSearch,
    ForgeFsUndo, ForgeFsWrite, ForgeImageRead, ForgePlanCreate, ForgeShell, ForgeSkillFetch,
};

type McpService<F> = ForgeMcpService<ForgeMcpManager<F>, F, <F as McpServerInfra>::Client>;
type AuthService<F> = ForgeAuthService<F>;

/// ForgeApp is the main application container that implements the App trait.
/// It provides access to all core services required by the application.
///
/// Type Parameters:
/// - F: The infrastructure implementation that provides core services like
///   environment, file reading, vector indexing, and embedding.
/// - R: The repository implementation that provides data persistence
#[derive(Clone)]
pub struct ForgeServices<
    F: HttpInfra
        + EnvironmentInfra
        + McpServerInfra
        + WalkerInfra
        + SnapshotRepository
        + ConversationRepository
        + KVStore
        + ChatRepository
        + ProviderRepository
        + WorkspaceIndexRepository
        + AgentRepository
        + SkillRepository
        + ValidationRepository,
> {
    chat_service: Arc<ForgeProviderService<F>>,
    config_service: Arc<ForgeAppConfigService<F>>,
    conversation_service: Arc<ForgeConversationService<F>>,
    template_service: Arc<ForgeTemplateService<F>>,
    attachment_service: Arc<ForgeChatRequest<F>>,
    discovery_service: Arc<ForgeDiscoveryService<F>>,
    mcp_manager: Arc<ForgeMcpManager<F>>,
    file_create_service: Arc<ForgeFsWrite<F>>,
    plan_create_service: Arc<ForgePlanCreate<F>>,
    file_read_service: Arc<ForgeFsRead<F>>,
    image_read_service: Arc<ForgeImageRead<F>>,
    file_search_service: Arc<ForgeFsSearch<F>>,
    file_remove_service: Arc<ForgeFsRemove<F>>,
    file_patch_service: Arc<ForgeFsPatch<F>>,
    file_undo_service: Arc<ForgeFsUndo<F>>,
    shell_service: Arc<ForgeShell<F>>,
    fetch_service: Arc<ForgeFetch>,
    followup_service: Arc<ForgeFollowup<F>>,
    mcp_service: Arc<McpService<F>>,
    custom_instructions_service: Arc<ForgeCustomInstructionsService<F>>,
    auth_service: Arc<AuthService<F>>,
    agent_registry_service: Arc<ForgeAgentRegistryService<F>>,
    command_loader_service: Arc<ForgeCommandLoaderService<F>>,
    policy_service: ForgePolicyService<F>,
    provider_auth_service: ForgeProviderAuthService<F>,
    workspace_service: Arc<crate::context_engine::ForgeWorkspaceService<F, FdDefault<F>>>,
    skill_service: Arc<ForgeSkillFetch<F>>,
    infra: Arc<F>,
}

impl<
    F: McpServerInfra
        + EnvironmentInfra<Config = forge_config::ForgeConfig>
        + FileWriterInfra
        + FileInfoInfra
        + FileReaderInfra
        + HttpInfra
        + WalkerInfra
        + DirectoryReaderInfra
        + CommandInfra
        + UserInfra
        + SnapshotRepository
        + ConversationRepository
        + ChatRepository
        + ProviderRepository
        + KVStore
        + WorkspaceIndexRepository
        + AgentRepository
        + SkillRepository
        + ValidationRepository,
> ForgeServices<F>
{
    pub fn new(infra: Arc<F>) -> Self {
        let mcp_manager = Arc::new(ForgeMcpManager::new(infra.clone()));
        let mcp_service = Arc::new(ForgeMcpService::new(mcp_manager.clone(), infra.clone()));
        let template_service = Arc::new(ForgeTemplateService::new(infra.clone()));
        let attachment_service = Arc::new(ForgeChatRequest::new(infra.clone()));
        let suggestion_service = Arc::new(ForgeDiscoveryService::new(infra.clone()));
        let conversation_service = Arc::new(ForgeConversationService::new(infra.clone()));
        let auth_service = Arc::new(ForgeAuthService::new(infra.clone()));
        let chat_service = Arc::new(ForgeProviderService::new(infra.clone()));
        let config_service = Arc::new(ForgeAppConfigService::new(infra.clone()));
        let file_create_service = Arc::new(ForgeFsWrite::new(infra.clone()));
        let plan_create_service = Arc::new(ForgePlanCreate::new(infra.clone()));
        let file_read_service = Arc::new(ForgeFsRead::new(infra.clone()));
        let image_read_service = Arc::new(ForgeImageRead::new(infra.clone()));
        let file_search_service = Arc::new(ForgeFsSearch::new(infra.clone()));
        let file_remove_service = Arc::new(ForgeFsRemove::new(infra.clone()));
        let file_patch_service = Arc::new(ForgeFsPatch::new(infra.clone()));
        let file_undo_service = Arc::new(ForgeFsUndo::new(infra.clone()));
        let shell_service = Arc::new(ForgeShell::new(infra.clone()));
        let fetch_service = Arc::new(ForgeFetch::new());
        let followup_service = Arc::new(ForgeFollowup::new(infra.clone()));
        let custom_instructions_service =
            Arc::new(ForgeCustomInstructionsService::new(infra.clone()));
        let agent_registry_service = Arc::new(ForgeAgentRegistryService::new(infra.clone()));
        let command_loader_service = Arc::new(ForgeCommandLoaderService::new(infra.clone()));
        let policy_service = ForgePolicyService::new(infra.clone());
        let provider_auth_service = ForgeProviderAuthService::new(infra.clone());
        let discovery = Arc::new(FdDefault::new(infra.clone()));
        let workspace_service = Arc::new(crate::context_engine::ForgeWorkspaceService::new(
            infra.clone(),
            discovery,
        ));
        let skill_service = Arc::new(ForgeSkillFetch::new(infra.clone()));

        Self {
            conversation_service,
            attachment_service,
            template_service,
            discovery_service: suggestion_service,
            mcp_manager,
            file_create_service,
            plan_create_service,
            file_read_service,
            image_read_service,
            file_search_service,
            file_remove_service,
            file_patch_service,
            file_undo_service,
            shell_service,
            fetch_service,
            followup_service,
            mcp_service,
            custom_instructions_service,
            auth_service,
            config_service,
            agent_registry_service,
            command_loader_service,
            policy_service,
            provider_auth_service,
            workspace_service,
            skill_service,
            chat_service,
            infra,
        }
    }
}

impl<
    F: FileReaderInfra
        + FileWriterInfra
        + CommandInfra
        + UserInfra
        + McpServerInfra
        + FileRemoverInfra
        + FileInfoInfra
        + FileDirectoryInfra
        + EnvironmentInfra<Config = forge_config::ForgeConfig>
        + DirectoryReaderInfra
        + HttpInfra
        + WalkerInfra
        + Clone
        + SnapshotRepository
        + ConversationRepository
        + KVStore
        + ChatRepository
        + ProviderRepository
        + AgentRepository
        + SkillRepository
        + StrategyFactory
        + WorkspaceIndexRepository
        + ValidationRepository
        + FuzzySearchRepository
        + Clone
        + 'static,
> Services for ForgeServices<F>
{
    type AppConfigService = ForgeAppConfigService<F>;
    type ConversationService = ForgeConversationService<F>;
    type TemplateService = ForgeTemplateService<F>;
    type ProviderAuthService = ForgeProviderAuthService<F>;

    fn provider_auth_service(&self) -> &Self::ProviderAuthService {
        &self.provider_auth_service
    }
    type AttachmentService = ForgeChatRequest<F>;
    type CustomInstructionsService = ForgeCustomInstructionsService<F>;
    type FileDiscoveryService = ForgeDiscoveryService<F>;
    type McpConfigManager = ForgeMcpManager<F>;
    type FsWriteService = ForgeFsWrite<F>;
    type PlanCreateService = ForgePlanCreate<F>;
    type FsPatchService = ForgeFsPatch<F>;
    type FsReadService = ForgeFsRead<F>;
    type ImageReadService = ForgeImageRead<F>;
    type FsRemoveService = ForgeFsRemove<F>;
    type FsSearchService = ForgeFsSearch<F>;
    type FollowUpService = ForgeFollowup<F>;
    type FsUndoService = ForgeFsUndo<F>;
    type NetFetchService = ForgeFetch;
    type ShellService = ForgeShell<F>;
    type McpService = McpService<F>;
    type AuthService = AuthService<F>;
    type AgentRegistry = ForgeAgentRegistryService<F>;
    type CommandLoaderService = ForgeCommandLoaderService<F>;
    type PolicyService = ForgePolicyService<F>;
    type ProviderService = ForgeProviderService<F>;
    type WorkspaceService = crate::context_engine::ForgeWorkspaceService<F, FdDefault<F>>;
    type SkillFetchService = ForgeSkillFetch<F>;

    fn config_service(&self) -> &Self::AppConfigService {
        &self.config_service
    }

    fn conversation_service(&self) -> &Self::ConversationService {
        &self.conversation_service
    }

    fn template_service(&self) -> &Self::TemplateService {
        &self.template_service
    }

    fn attachment_service(&self) -> &Self::AttachmentService {
        &self.attachment_service
    }

    fn custom_instructions_service(&self) -> &Self::CustomInstructionsService {
        &self.custom_instructions_service
    }

    fn file_discovery_service(&self) -> &Self::FileDiscoveryService {
        self.discovery_service.as_ref()
    }

    fn mcp_config_manager(&self) -> &Self::McpConfigManager {
        self.mcp_manager.as_ref()
    }

    fn fs_create_service(&self) -> &Self::FsWriteService {
        &self.file_create_service
    }

    fn plan_create_service(&self) -> &Self::PlanCreateService {
        &self.plan_create_service
    }

    fn fs_patch_service(&self) -> &Self::FsPatchService {
        &self.file_patch_service
    }

    fn fs_read_service(&self) -> &Self::FsReadService {
        &self.file_read_service
    }

    fn fs_remove_service(&self) -> &Self::FsRemoveService {
        &self.file_remove_service
    }

    fn fs_search_service(&self) -> &Self::FsSearchService {
        &self.file_search_service
    }

    fn follow_up_service(&self) -> &Self::FollowUpService {
        &self.followup_service
    }

    fn fs_undo_service(&self) -> &Self::FsUndoService {
        &self.file_undo_service
    }

    fn net_fetch_service(&self) -> &Self::NetFetchService {
        &self.fetch_service
    }

    fn shell_service(&self) -> &Self::ShellService {
        &self.shell_service
    }

    fn mcp_service(&self) -> &Self::McpService {
        &self.mcp_service
    }

    fn auth_service(&self) -> &Self::AuthService {
        self.auth_service.as_ref()
    }

    fn agent_registry(&self) -> &Self::AgentRegistry {
        &self.agent_registry_service
    }

    fn command_loader_service(&self) -> &Self::CommandLoaderService {
        &self.command_loader_service
    }

    fn policy_service(&self) -> &Self::PolicyService {
        &self.policy_service
    }

    fn workspace_service(&self) -> &Self::WorkspaceService {
        &self.workspace_service
    }

    fn image_read_service(&self) -> &Self::ImageReadService {
        &self.image_read_service
    }
    fn skill_fetch_service(&self) -> &Self::SkillFetchService {
        &self.skill_service
    }

    fn provider_service(&self) -> &Self::ProviderService {
        &self.chat_service
    }
}

impl<
    F: EnvironmentInfra<Config = forge_config::ForgeConfig>
        + HttpInfra
        + McpServerInfra
        + WalkerInfra
        + SnapshotRepository
        + ConversationRepository
        + KVStore
        + ChatRepository
        + ProviderRepository
        + WorkspaceIndexRepository
        + AgentRepository
        + SkillRepository
        + ValidationRepository
        + Send
        + Sync,
> forge_app::EnvironmentInfra for ForgeServices<F>
{
    type Config = forge_config::ForgeConfig;

    fn get_environment(&self) -> forge_domain::Environment {
        self.infra.get_environment()
    }

    fn get_config(&self) -> anyhow::Result<forge_config::ForgeConfig> {
        self.infra.get_config()
    }

    fn update_environment(
        &self,
        ops: Vec<forge_domain::ConfigOperation>,
    ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send {
        self.infra.update_environment(ops)
    }

    fn get_env_var(&self, key: &str) -> Option<String> {
        self.infra.get_env_var(key)
    }

    fn get_env_vars(&self) -> std::collections::BTreeMap<String, String> {
        self.infra.get_env_vars()
    }
}
