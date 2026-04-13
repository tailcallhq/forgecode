use std::path::{Path, PathBuf};
use std::time::Duration;

use bytes::Bytes;
use derive_setters::Setters;
use forge_domain::{
    AgentId, AnyProvider, Attachment, AuthContextRequest, AuthContextResponse, AuthMethod,
    ChatCompletionMessage, CommandOutput, Context, Conversation, ConversationId, File, FileInfo,
    FileStatus, Image, McpConfig, McpServers, Model, ModelId, Node, Provider, ProviderId,
    ResultStream, Scope, SearchParams, SyncProgress, SyntaxError, Template, ToolCallFull,
    ToolOutput, WorkspaceAuth, WorkspaceId, WorkspaceInfo,
};
use reqwest::Response;
use reqwest::header::HeaderMap;
use reqwest_eventsource::EventSource;
use url::Url;

use crate::user::{User, UserUsage};
use crate::{EnvironmentInfra, Walker};

#[derive(Debug, Clone)]
pub struct ShellOutput {
    pub output: CommandOutput,
    pub shell: String,
    pub description: Option<String>,
}

#[derive(Debug)]
pub struct PatchOutput {
    pub errors: Vec<SyntaxError>,
    pub before: String,
    pub after: String,
    pub content_hash: String,
}

#[derive(Debug, Setters)]
#[setters(into)]
pub struct ReadOutput {
    pub content: Content,
    pub info: FileInfo,
}

#[derive(Debug)]
pub enum Content {
    File(String),
    Image(Image),
}

impl Content {
    pub fn file<S: Into<String>>(content: S) -> Self {
        Self::File(content.into())
    }

    pub fn image(image: Image) -> Self {
        Self::Image(image)
    }

    pub fn file_content(&self) -> &str {
        match self {
            Self::File(content) => content,
            Self::Image(_) => "",
        }
    }

    pub fn as_image(&self) -> Option<&Image> {
        match self {
            Self::Image(img) => Some(img),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct SearchResult {
    pub matches: Vec<Match>,
}

#[derive(Debug)]
pub struct Match {
    pub path: String,
    pub result: Option<MatchResult>,
}

#[derive(Debug)]
pub enum MatchResult {
    Error(String),
    Found {
        line_number: Option<usize>,
        line: String,
    },
    Count {
        count: usize,
    },
    FileMatch, // For files_with_matches mode
    ContextMatch {
        line_number: Option<usize>,
        line: String,
        before_context: Vec<String>,
        after_context: Vec<String>,
    },
}

#[derive(Debug)]
pub struct HttpResponse {
    pub content: String,
    pub code: u16,
    pub context: ResponseContext,
    pub content_type: String,
}

#[derive(Debug)]
pub enum ResponseContext {
    Parsed,
    Raw,
}

#[derive(Debug)]
pub struct FsWriteOutput {
    pub path: String,
    // Set when the file already exists
    pub before: Option<String>,
    pub errors: Vec<SyntaxError>,
    pub content_hash: String,
}

#[derive(Debug)]
pub struct FsRemoveOutput {
    // Content of the file
    pub content: String,
}

#[derive(Debug)]
pub struct PlanCreateOutput {
    pub path: PathBuf,
    // Set when the file already exists
    pub before: Option<String>,
}

#[derive(Default, Debug, derive_more::From)]
pub struct FsUndoOutput {
    pub before_undo: Option<String>,
    pub after_undo: Option<String>,
}

/// Output from todo_write tool execution
#[derive(Debug)]
pub struct TodoWriteOutput {
    /// List of todos that were saved
    pub todos: Vec<forge_domain::Todo>,
}

#[derive(Debug)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub path: Option<PathBuf>,
}

#[async_trait::async_trait]
pub trait ProviderService: Send + Sync {
    async fn chat(
        &self,
        model_id: &ModelId,
        context: Context,
        provider: Provider<Url>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error>;
    async fn models(&self, provider: Provider<Url>) -> anyhow::Result<Vec<Model>>;
    async fn get_provider(&self, id: forge_domain::ProviderId) -> anyhow::Result<Provider<Url>>;
    async fn get_all_providers(&self) -> anyhow::Result<Vec<AnyProvider>>;
    async fn upsert_credential(
        &self,
        credential: forge_domain::AuthCredential,
    ) -> anyhow::Result<()>;
    async fn remove_credential(&self, id: &forge_domain::ProviderId) -> anyhow::Result<()>;
    /// Migrates environment variable-based credentials to file-based
    /// credentials. Returns Some(MigrationResult) if credentials were migrated,
    /// None if file already exists or no credentials to migrate.
    async fn migrate_env_credentials(
        &self,
    ) -> anyhow::Result<Option<forge_domain::MigrationResult>>;
}
/// Manages user preferences for default providers and models.
#[async_trait::async_trait]
pub trait AppConfigService: Send + Sync {
    /// Gets the user's default provider ID.
    async fn get_default_provider(&self) -> anyhow::Result<ProviderId>;

    /// Gets the user's default model for a specific provider or the currently
    /// active provider. When provider_id is None, uses the currently active
    /// provider.
    ///
    /// # Errors
    /// - Returns `Error::NoDefaultSession` when no provider and model are
    ///   configured.
    async fn get_provider_model(
        &self,
        provider_id: Option<&forge_domain::ProviderId>,
    ) -> anyhow::Result<ModelId>;

    /// Gets the commit configuration (provider and model for commit message
    /// generation).
    async fn get_commit_config(&self) -> anyhow::Result<Option<forge_domain::ModelConfig>>;

    /// Gets the suggest configuration (provider and model for command
    /// suggestion generation).
    async fn get_suggest_config(&self) -> anyhow::Result<Option<forge_domain::ModelConfig>>;

    /// Gets the current reasoning effort setting.
    async fn get_reasoning_effort(&self) -> anyhow::Result<Option<forge_domain::Effort>>;

    /// Applies one or more configuration mutations atomically.
    ///
    /// Each operation in `ops` is applied in order, and the result is
    /// persisted as a single atomic write. This is the sole write path for
    /// all configuration changes; use [`forge_domain::ConfigOperation`]
    /// variants to describe each mutation.
    async fn update_config(&self, ops: Vec<forge_domain::ConfigOperation>) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
pub trait McpConfigManager: Send + Sync {
    /// Responsible to load the MCP servers from all configuration files.
    /// If scope is provided, only loads from that specific scope (not merged).
    async fn read_mcp_config(&self, scope: Option<&Scope>) -> anyhow::Result<McpConfig>;

    /// Responsible for writing the McpConfig on disk.
    async fn write_mcp_config(&self, config: &McpConfig, scope: &Scope) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
pub trait McpService: Send + Sync {
    async fn get_mcp_servers(&self) -> anyhow::Result<McpServers>;
    async fn execute_mcp(&self, call: ToolCallFull) -> anyhow::Result<ToolOutput>;
    /// Refresh the MCP cache by fetching fresh data
    async fn reload_mcp(&self) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
pub trait ConversationService: Send + Sync {
    async fn find_conversation(&self, id: &ConversationId) -> anyhow::Result<Option<Conversation>>;

    async fn upsert_conversation(&self, conversation: Conversation) -> anyhow::Result<()>;

    /// This is useful when you want to perform several operations on a
    /// conversation atomically.
    async fn modify_conversation<F, T>(&self, id: &ConversationId, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&mut Conversation) -> T + Send,
        T: Send;

    /// Find conversations with optional limit
    async fn get_conversations(
        &self,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<Conversation>>>;

    /// Find the last active conversation
    async fn last_conversation(&self) -> anyhow::Result<Option<Conversation>>;

    /// Permanently deletes a conversation
    async fn delete_conversation(&self, conversation_id: &ConversationId) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
pub trait TemplateService: Send + Sync {
    async fn register_template(&self, path: PathBuf) -> anyhow::Result<()>;
    async fn render_template<V: serde::Serialize + Send + Sync>(
        &self,
        template: Template<V>,
        object: &V,
    ) -> anyhow::Result<String>;
}

#[async_trait::async_trait]
pub trait AttachmentService {
    async fn attachments(&self, url: &str) -> anyhow::Result<Vec<Attachment>>;
}

#[async_trait::async_trait]
pub trait CustomInstructionsService: Send + Sync {
    async fn get_custom_instructions(&self) -> Vec<String>;
}

/// Service for indexing workspaces for semantic search
#[async_trait::async_trait]
pub trait WorkspaceService: Send + Sync {
    /// Index the workspace at the given path
    async fn sync_workspace(
        &self,
        path: PathBuf,
    ) -> anyhow::Result<forge_stream::MpscStream<anyhow::Result<SyncProgress>>>;

    /// Query the indexed workspace with semantic search
    async fn query_workspace(
        &self,
        path: PathBuf,
        params: SearchParams<'_>,
    ) -> anyhow::Result<Vec<Node>>;

    /// List all workspaces indexed by the user
    async fn list_workspaces(&self) -> anyhow::Result<Vec<WorkspaceInfo>>;

    /// Get workspace information for a specific path
    async fn get_workspace_info(&self, path: PathBuf) -> anyhow::Result<Option<WorkspaceInfo>>;

    /// Delete a workspace and all its indexed data
    async fn delete_workspace(&self, workspace_id: &WorkspaceId) -> anyhow::Result<()>;

    /// Delete multiple workspaces in parallel and all their indexed data
    async fn delete_workspaces(&self, workspace_ids: &[WorkspaceId]) -> anyhow::Result<()>;

    /// Checks if workspace is indexed.
    async fn is_indexed(&self, path: &Path) -> anyhow::Result<bool>;

    /// Get sync status for all files in workspace
    async fn get_workspace_status(&self, path: PathBuf) -> anyhow::Result<Vec<FileStatus>>;

    /// Check if authentication credentials exist
    async fn is_authenticated(&self) -> anyhow::Result<bool>;

    /// Create new authentication credentials
    async fn init_auth_credentials(&self) -> anyhow::Result<WorkspaceAuth>;

    /// Initialize a workspace without syncing files
    async fn init_workspace(&self, path: PathBuf) -> anyhow::Result<WorkspaceId>;
}

#[async_trait::async_trait]
pub trait FileDiscoveryService: Send + Sync {
    async fn collect_files(&self, config: Walker) -> anyhow::Result<Vec<File>>;

    /// Lists all entries (files and directories) in the current directory
    /// Returns a sorted vector of File entries with directories first
    async fn list_current_directory(&self) -> anyhow::Result<Vec<File>>;
}

#[async_trait::async_trait]
pub trait FsWriteService: Send + Sync {
    /// Create a file at the specified path with the given content.
    async fn write(
        &self,
        path: String,
        content: String,
        overwrite: bool,
    ) -> anyhow::Result<FsWriteOutput>;
}

#[async_trait::async_trait]
pub trait PlanCreateService: Send + Sync {
    /// Create a plan file with the specified name and version.
    async fn create_plan(
        &self,
        plan_name: String,
        version: String,
        content: String,
    ) -> anyhow::Result<PlanCreateOutput>;
}

#[async_trait::async_trait]
pub trait FsPatchService: Send + Sync {
    /// Patches a file at the specified path with the given content.
    async fn patch(
        &self,
        path: String,
        search: String,
        content: String,
        replace_all: bool,
    ) -> anyhow::Result<PatchOutput>;

    /// Applies multiple patches to a single file in sequence
    async fn multi_patch(
        &self,
        path: String,
        edits: Vec<forge_domain::PatchEdit>,
    ) -> anyhow::Result<PatchOutput>;
}

#[async_trait::async_trait]
pub trait FsReadService: Send + Sync {
    /// Reads a file at the specified path and returns its content.
    async fn read(
        &self,
        path: String,
        start_line: Option<u64>,
        end_line: Option<u64>,
    ) -> anyhow::Result<ReadOutput>;
}

#[async_trait::async_trait]
pub trait ImageReadService: Send + Sync {
    /// Reads an image file at the specified path and returns its content.
    async fn read_image(&self, path: String) -> anyhow::Result<forge_domain::Image>;
}

#[async_trait::async_trait]
pub trait FsRemoveService: Send + Sync {
    /// Removes a file at the specified path.
    async fn remove(&self, path: String) -> anyhow::Result<FsRemoveOutput>;
}

#[async_trait::async_trait]
pub trait FsSearchService: Send + Sync {
    /// Searches for files and content based on the provided parameters.
    ///
    /// # Arguments
    /// * `params` - Search parameters including pattern, path, output mode,
    ///   etc.
    ///
    /// # Returns
    /// * `Ok(Some(SearchResult))` - Matches found
    /// * `Ok(None)` - No matches found
    /// * `Err(_)` - Search error
    async fn search(&self, params: forge_domain::FSSearch) -> anyhow::Result<Option<SearchResult>>;
}

#[async_trait::async_trait]
pub trait FollowUpService: Send + Sync {
    /// Follows up on a tool call with the given context.
    async fn follow_up(
        &self,
        question: String,
        options: Vec<String>,
        multiple: Option<bool>,
    ) -> anyhow::Result<Option<String>>;
}

#[async_trait::async_trait]
pub trait FsUndoService: Send + Sync {
    /// Undoes the last file operation at the specified path.
    /// And returns the content of the undone file.
    // TODO: We should move Snapshot service to Services from infra
    // and drop FsUndoService.
    async fn undo(&self, path: String) -> anyhow::Result<FsUndoOutput>;
}

#[async_trait::async_trait]
pub trait NetFetchService: Send + Sync {
    /// Fetches content from a URL and returns it as a string.
    async fn fetch(&self, url: String, raw: Option<bool>) -> anyhow::Result<HttpResponse>;
}

#[async_trait::async_trait]
pub trait ShellService: Send + Sync {
    /// Executes a shell command and returns the output.
    async fn execute(
        &self,
        command: String,
        cwd: PathBuf,
        keep_ansi: bool,
        silent: bool,
        env_vars: Option<Vec<String>>,
        description: Option<String>,
    ) -> anyhow::Result<ShellOutput>;
}

#[async_trait::async_trait]
pub trait AuthService: Send + Sync {
    async fn user_info(&self, api_key: &str) -> anyhow::Result<User>;
    async fn user_usage(&self, api_key: &str) -> anyhow::Result<UserUsage>;
}

#[async_trait::async_trait]
pub trait AgentRegistry: Send + Sync {
    /// Get the active agent ID
    async fn get_active_agent_id(&self) -> anyhow::Result<Option<AgentId>>;

    /// Set the active agent ID
    async fn set_active_agent_id(&self, agent_id: AgentId) -> anyhow::Result<()>;

    /// Get all agents from the registry store
    async fn get_agents(&self) -> anyhow::Result<Vec<forge_domain::Agent>>;

    /// Get lightweight metadata for all agents without requiring a configured
    /// provider or model
    async fn get_agent_infos(&self) -> anyhow::Result<Vec<forge_domain::AgentInfo>>;

    /// Get agent by ID (from registry store)
    async fn get_agent(&self, agent_id: &AgentId) -> anyhow::Result<Option<forge_domain::Agent>>;

    /// Reload agents by invalidating the cache
    async fn reload_agents(&self) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
pub trait CommandLoaderService: Send + Sync {
    /// Load all command definitions from the forge/commands directory
    async fn get_commands(&self) -> anyhow::Result<Vec<forge_domain::Command>>;
}

#[async_trait::async_trait]
pub trait PolicyService: Send + Sync {
    /// Check if an operation is allowed and handle user confirmation if needed
    /// Returns PolicyDecision with allowed flag and optional policy file path
    /// (only when created)
    async fn check_operation_permission(
        &self,
        operation: &forge_domain::PermissionOperation,
    ) -> anyhow::Result<PolicyDecision>;
}

/// Skill fetch service
#[async_trait::async_trait]
pub trait SkillFetchService: Send + Sync {
    /// Fetches a skill by name
    ///
    /// # Errors
    ///
    /// Returns an error if the skill is not found or cannot be loaded
    async fn fetch_skill(&self, skill_name: String) -> anyhow::Result<forge_domain::Skill>;

    /// Lists all available skills
    ///
    /// # Errors
    ///
    /// Returns an error if skills cannot be loaded
    async fn list_skills(&self) -> anyhow::Result<Vec<forge_domain::Skill>>;
}

/// Provider authentication service
#[async_trait::async_trait]
pub trait ProviderAuthService: Send + Sync {
    async fn init_provider_auth(
        &self,
        provider_id: ProviderId,
        method: AuthMethod,
    ) -> anyhow::Result<AuthContextRequest>;
    async fn complete_provider_auth(
        &self,
        provider_id: ProviderId,
        context: AuthContextResponse,
        timeout: Duration,
    ) -> anyhow::Result<()>;

    /// Refreshes provider credentials if they're about to expire.
    /// Checks if credential needs refresh (5 minute buffer before expiry),
    /// iterates through provider's auth methods, and attempts to refresh.
    /// Returns the provider with updated credentials, or original if refresh
    /// fails or isn't needed.
    async fn refresh_provider_credential(
        &self,
        provider: Provider<Url>,
    ) -> anyhow::Result<Provider<Url>>;
}

pub trait Services: Send + Sync + 'static + Clone + EnvironmentInfra {
    type ProviderService: ProviderService;
    type AppConfigService: AppConfigService;
    type ConversationService: ConversationService;
    type TemplateService: TemplateService;
    type AttachmentService: AttachmentService;
    type CustomInstructionsService: CustomInstructionsService;
    type FileDiscoveryService: FileDiscoveryService;
    type McpConfigManager: McpConfigManager;
    type FsWriteService: FsWriteService;
    type PlanCreateService: PlanCreateService;
    type FsPatchService: FsPatchService;
    type FsReadService: FsReadService;
    type ImageReadService: ImageReadService;
    type FsRemoveService: FsRemoveService;
    type FsSearchService: FsSearchService;
    type FollowUpService: FollowUpService;
    type FsUndoService: FsUndoService;
    type NetFetchService: NetFetchService;
    type ShellService: ShellService;
    type McpService: McpService;
    type AuthService: AuthService;
    type AgentRegistry: AgentRegistry;
    type CommandLoaderService: CommandLoaderService;
    type PolicyService: PolicyService;
    type ProviderAuthService: ProviderAuthService;
    type WorkspaceService: WorkspaceService;
    type SkillFetchService: SkillFetchService;

    fn provider_service(&self) -> &Self::ProviderService;
    fn config_service(&self) -> &Self::AppConfigService;
    fn conversation_service(&self) -> &Self::ConversationService;
    fn template_service(&self) -> &Self::TemplateService;
    fn attachment_service(&self) -> &Self::AttachmentService;
    fn file_discovery_service(&self) -> &Self::FileDiscoveryService;
    fn mcp_config_manager(&self) -> &Self::McpConfigManager;
    fn fs_create_service(&self) -> &Self::FsWriteService;
    fn plan_create_service(&self) -> &Self::PlanCreateService;
    fn fs_patch_service(&self) -> &Self::FsPatchService;
    fn fs_read_service(&self) -> &Self::FsReadService;
    fn image_read_service(&self) -> &Self::ImageReadService;
    fn fs_remove_service(&self) -> &Self::FsRemoveService;
    fn fs_search_service(&self) -> &Self::FsSearchService;
    fn follow_up_service(&self) -> &Self::FollowUpService;
    fn fs_undo_service(&self) -> &Self::FsUndoService;
    fn net_fetch_service(&self) -> &Self::NetFetchService;
    fn shell_service(&self) -> &Self::ShellService;
    fn mcp_service(&self) -> &Self::McpService;
    fn custom_instructions_service(&self) -> &Self::CustomInstructionsService;
    fn auth_service(&self) -> &Self::AuthService;
    fn agent_registry(&self) -> &Self::AgentRegistry;
    fn command_loader_service(&self) -> &Self::CommandLoaderService;
    fn policy_service(&self) -> &Self::PolicyService;
    fn provider_auth_service(&self) -> &Self::ProviderAuthService;
    fn workspace_service(&self) -> &Self::WorkspaceService;
    fn skill_fetch_service(&self) -> &Self::SkillFetchService;
}

#[async_trait::async_trait]
impl<I: Services> ConversationService for I {
    async fn find_conversation(&self, id: &ConversationId) -> anyhow::Result<Option<Conversation>> {
        self.conversation_service().find_conversation(id).await
    }

    async fn upsert_conversation(&self, conversation: Conversation) -> anyhow::Result<()> {
        self.conversation_service()
            .upsert_conversation(conversation)
            .await
    }

    async fn modify_conversation<F, T>(&self, id: &ConversationId, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&mut Conversation) -> T + Send,
        T: Send,
    {
        self.conversation_service().modify_conversation(id, f).await
    }

    async fn get_conversations(
        &self,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        self.conversation_service().get_conversations(limit).await
    }

    async fn last_conversation(&self) -> anyhow::Result<Option<Conversation>> {
        self.conversation_service().last_conversation().await
    }

    async fn delete_conversation(&self, conversation_id: &ConversationId) -> anyhow::Result<()> {
        self.conversation_service()
            .delete_conversation(conversation_id)
            .await
    }
}
#[async_trait::async_trait]
impl<I: Services> ProviderService for I {
    async fn chat(
        &self,
        model_id: &ModelId,
        context: Context,
        provider: Provider<Url>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        self.provider_service()
            .chat(model_id, context, provider)
            .await
    }

    async fn models(&self, provider: Provider<Url>) -> anyhow::Result<Vec<Model>> {
        self.provider_service().models(provider).await
    }

    async fn get_provider(&self, id: forge_domain::ProviderId) -> anyhow::Result<Provider<Url>> {
        self.provider_service().get_provider(id).await
    }

    async fn get_all_providers(&self) -> anyhow::Result<Vec<AnyProvider>> {
        self.provider_service().get_all_providers().await
    }

    async fn upsert_credential(
        &self,
        credential: forge_domain::AuthCredential,
    ) -> anyhow::Result<()> {
        self.provider_service().upsert_credential(credential).await
    }

    async fn remove_credential(&self, id: &forge_domain::ProviderId) -> anyhow::Result<()> {
        self.provider_service().remove_credential(id).await
    }

    async fn migrate_env_credentials(
        &self,
    ) -> anyhow::Result<Option<forge_domain::MigrationResult>> {
        self.provider_service().migrate_env_credentials().await
    }
}

#[async_trait::async_trait]
impl<I: Services> McpConfigManager for I {
    async fn read_mcp_config(&self, scope: Option<&Scope>) -> anyhow::Result<McpConfig> {
        self.mcp_config_manager().read_mcp_config(scope).await
    }

    async fn write_mcp_config(&self, config: &McpConfig, scope: &Scope) -> anyhow::Result<()> {
        self.mcp_config_manager()
            .write_mcp_config(config, scope)
            .await
    }
}

#[async_trait::async_trait]
impl<I: Services> McpService for I {
    async fn get_mcp_servers(&self) -> anyhow::Result<McpServers> {
        self.mcp_service().get_mcp_servers().await
    }

    async fn execute_mcp(&self, call: ToolCallFull) -> anyhow::Result<ToolOutput> {
        self.mcp_service().execute_mcp(call).await
    }

    async fn reload_mcp(&self) -> anyhow::Result<()> {
        self.mcp_service().reload_mcp().await
    }
}

#[async_trait::async_trait]
impl<I: Services> TemplateService for I {
    async fn register_template(&self, path: PathBuf) -> anyhow::Result<()> {
        self.template_service().register_template(path).await
    }

    async fn render_template<V: serde::Serialize + Send + Sync>(
        &self,
        template: Template<V>,
        object: &V,
    ) -> anyhow::Result<String> {
        self.template_service()
            .render_template(template, object)
            .await
    }
}

#[async_trait::async_trait]
impl<I: Services> AttachmentService for I {
    async fn attachments(&self, url: &str) -> anyhow::Result<Vec<Attachment>> {
        self.attachment_service().attachments(url).await
    }
}

#[async_trait::async_trait]
impl<I: Services> FileDiscoveryService for I {
    async fn collect_files(&self, config: Walker) -> anyhow::Result<Vec<File>> {
        self.file_discovery_service().collect_files(config).await
    }

    async fn list_current_directory(&self) -> anyhow::Result<Vec<File>> {
        self.file_discovery_service().list_current_directory().await
    }
}

#[async_trait::async_trait]
impl<I: Services> FsWriteService for I {
    async fn write(
        &self,
        path: String,
        content: String,
        overwrite: bool,
    ) -> anyhow::Result<FsWriteOutput> {
        self.fs_create_service()
            .write(path, content, overwrite)
            .await
    }
}

#[async_trait::async_trait]
impl<I: Services> PlanCreateService for I {
    async fn create_plan(
        &self,
        plan_name: String,
        version: String,
        content: String,
    ) -> anyhow::Result<PlanCreateOutput> {
        self.plan_create_service()
            .create_plan(plan_name, version, content)
            .await
    }
}

#[async_trait::async_trait]
impl<I: Services> FsPatchService for I {
    async fn patch(
        &self,
        path: String,
        search: String,
        content: String,
        replace_all: bool,
    ) -> anyhow::Result<PatchOutput> {
        self.fs_patch_service()
            .patch(path, search, content, replace_all)
            .await
    }

    async fn multi_patch(
        &self,
        path: String,
        edits: Vec<forge_domain::PatchEdit>,
    ) -> anyhow::Result<PatchOutput> {
        self.fs_patch_service().multi_patch(path, edits).await
    }
}

#[async_trait::async_trait]
impl<I: Services> FsReadService for I {
    async fn read(
        &self,
        path: String,
        start_line: Option<u64>,
        end_line: Option<u64>,
    ) -> anyhow::Result<ReadOutput> {
        self.fs_read_service()
            .read(path, start_line, end_line)
            .await
    }
}
#[async_trait::async_trait]
impl<I: Services> ImageReadService for I {
    async fn read_image(&self, path: String) -> anyhow::Result<Image> {
        self.image_read_service().read_image(path).await
    }
}

#[async_trait::async_trait]
impl<I: Services> FsRemoveService for I {
    async fn remove(&self, path: String) -> anyhow::Result<FsRemoveOutput> {
        self.fs_remove_service().remove(path).await
    }
}

#[async_trait::async_trait]
impl<I: Services> FsSearchService for I {
    async fn search(&self, params: forge_domain::FSSearch) -> anyhow::Result<Option<SearchResult>> {
        self.fs_search_service().search(params).await
    }
}

#[async_trait::async_trait]
impl<I: Services> FollowUpService for I {
    async fn follow_up(
        &self,
        question: String,
        options: Vec<String>,
        multiple: Option<bool>,
    ) -> anyhow::Result<Option<String>> {
        self.follow_up_service()
            .follow_up(question, options, multiple)
            .await
    }
}

#[async_trait::async_trait]
impl<I: Services> FsUndoService for I {
    async fn undo(&self, path: String) -> anyhow::Result<FsUndoOutput> {
        self.fs_undo_service().undo(path).await
    }
}

#[async_trait::async_trait]
impl<I: Services> NetFetchService for I {
    async fn fetch(&self, url: String, raw: Option<bool>) -> anyhow::Result<HttpResponse> {
        self.net_fetch_service().fetch(url, raw).await
    }
}

#[async_trait::async_trait]
impl<I: Services> ShellService for I {
    async fn execute(
        &self,
        command: String,
        cwd: PathBuf,
        keep_ansi: bool,
        silent: bool,
        env_vars: Option<Vec<String>>,
        description: Option<String>,
    ) -> anyhow::Result<ShellOutput> {
        self.shell_service()
            .execute(command, cwd, keep_ansi, silent, env_vars, description)
            .await
    }
}

#[async_trait::async_trait]
impl<I: Services> CustomInstructionsService for I {
    async fn get_custom_instructions(&self) -> Vec<String> {
        self.custom_instructions_service()
            .get_custom_instructions()
            .await
    }
}

#[async_trait::async_trait]
impl<I: Services> AuthService for I {
    async fn user_info(&self, api_key: &str) -> anyhow::Result<User> {
        self.auth_service().user_info(api_key).await
    }

    async fn user_usage(&self, api_key: &str) -> anyhow::Result<UserUsage> {
        self.auth_service().user_usage(api_key).await
    }
}

/// HTTP service trait for making HTTP requests
#[async_trait::async_trait]
pub trait HttpClientService: Send + Sync + 'static {
    async fn get(&self, url: &Url, headers: Option<HeaderMap>) -> anyhow::Result<Response>;
    async fn post(&self, url: &Url, body: bytes::Bytes) -> anyhow::Result<Response>;
    async fn delete(&self, url: &Url) -> anyhow::Result<Response>;

    /// Posts JSON data and returns a server-sent events stream
    async fn eventsource(
        &self,
        url: &Url,
        headers: Option<HeaderMap>,
        body: Bytes,
    ) -> anyhow::Result<EventSource>;
}

#[async_trait::async_trait]
impl<I: Services> AgentRegistry for I {
    async fn get_active_agent_id(&self) -> anyhow::Result<Option<AgentId>> {
        self.agent_registry().get_active_agent_id().await
    }

    async fn set_active_agent_id(&self, agent_id: AgentId) -> anyhow::Result<()> {
        self.agent_registry().set_active_agent_id(agent_id).await
    }

    async fn get_agents(&self) -> anyhow::Result<Vec<forge_domain::Agent>> {
        self.agent_registry().get_agents().await
    }

    async fn get_agent_infos(&self) -> anyhow::Result<Vec<forge_domain::AgentInfo>> {
        self.agent_registry().get_agent_infos().await
    }

    async fn get_agent(&self, agent_id: &AgentId) -> anyhow::Result<Option<forge_domain::Agent>> {
        self.agent_registry().get_agent(agent_id).await
    }

    async fn reload_agents(&self) -> anyhow::Result<()> {
        self.agent_registry().reload_agents().await
    }
}

#[async_trait::async_trait]
impl<I: Services> CommandLoaderService for I {
    async fn get_commands(&self) -> anyhow::Result<Vec<forge_domain::Command>> {
        self.command_loader_service().get_commands().await
    }
}

#[async_trait::async_trait]
impl<I: Services> PolicyService for I {
    async fn check_operation_permission(
        &self,
        operation: &forge_domain::PermissionOperation,
    ) -> anyhow::Result<PolicyDecision> {
        self.policy_service()
            .check_operation_permission(operation)
            .await
    }
}

#[async_trait::async_trait]
impl<I: Services> AppConfigService for I {
    async fn get_default_provider(&self) -> anyhow::Result<ProviderId> {
        self.config_service().get_default_provider().await
    }

    async fn get_provider_model(
        &self,
        provider_id: Option<&forge_domain::ProviderId>,
    ) -> anyhow::Result<ModelId> {
        self.config_service().get_provider_model(provider_id).await
    }

    async fn get_commit_config(&self) -> anyhow::Result<Option<forge_domain::ModelConfig>> {
        self.config_service().get_commit_config().await
    }

    async fn get_suggest_config(&self) -> anyhow::Result<Option<forge_domain::ModelConfig>> {
        self.config_service().get_suggest_config().await
    }

    async fn get_reasoning_effort(&self) -> anyhow::Result<Option<forge_domain::Effort>> {
        self.config_service().get_reasoning_effort().await
    }

    async fn update_config(&self, ops: Vec<forge_domain::ConfigOperation>) -> anyhow::Result<()> {
        self.config_service().update_config(ops).await
    }
}

#[async_trait::async_trait]
impl<I: Services> SkillFetchService for I {
    async fn fetch_skill(&self, skill_name: String) -> anyhow::Result<forge_domain::Skill> {
        self.skill_fetch_service().fetch_skill(skill_name).await
    }

    async fn list_skills(&self) -> anyhow::Result<Vec<forge_domain::Skill>> {
        self.skill_fetch_service().list_skills().await
    }
}

#[async_trait::async_trait]
impl<I: Services> ProviderAuthService for I {
    async fn init_provider_auth(
        &self,
        provider_id: ProviderId,
        method: AuthMethod,
    ) -> anyhow::Result<AuthContextRequest> {
        self.provider_auth_service()
            .init_provider_auth(provider_id, method)
            .await
    }
    async fn complete_provider_auth(
        &self,
        provider_id: ProviderId,
        context: AuthContextResponse,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        self.provider_auth_service()
            .complete_provider_auth(provider_id, context, timeout)
            .await
    }
    async fn refresh_provider_credential(
        &self,
        provider: Provider<Url>,
    ) -> anyhow::Result<Provider<Url>> {
        self.provider_auth_service()
            .refresh_provider_credential(provider)
            .await
    }
}

#[async_trait::async_trait]
impl<I: Services> WorkspaceService for I {
    async fn sync_workspace(
        &self,
        path: PathBuf,
    ) -> anyhow::Result<forge_stream::MpscStream<anyhow::Result<SyncProgress>>> {
        self.workspace_service().sync_workspace(path).await
    }

    async fn query_workspace(
        &self,
        path: PathBuf,
        params: SearchParams<'_>,
    ) -> anyhow::Result<Vec<Node>> {
        self.workspace_service().query_workspace(path, params).await
    }

    async fn list_workspaces(&self) -> anyhow::Result<Vec<WorkspaceInfo>> {
        self.workspace_service().list_workspaces().await
    }

    async fn get_workspace_info(&self, path: PathBuf) -> anyhow::Result<Option<WorkspaceInfo>> {
        self.workspace_service().get_workspace_info(path).await
    }

    async fn delete_workspace(&self, workspace_id: &WorkspaceId) -> anyhow::Result<()> {
        self.workspace_service()
            .delete_workspace(workspace_id)
            .await
    }

    async fn delete_workspaces(&self, workspace_ids: &[WorkspaceId]) -> anyhow::Result<()> {
        self.workspace_service()
            .delete_workspaces(workspace_ids)
            .await
    }

    async fn is_indexed(&self, path: &Path) -> anyhow::Result<bool> {
        self.workspace_service().is_indexed(path).await
    }

    async fn get_workspace_status(&self, path: PathBuf) -> anyhow::Result<Vec<FileStatus>> {
        self.workspace_service().get_workspace_status(path).await
    }

    async fn is_authenticated(&self) -> anyhow::Result<bool> {
        self.workspace_service().is_authenticated().await
    }

    async fn init_auth_credentials(&self) -> anyhow::Result<WorkspaceAuth> {
        self.workspace_service().init_auth_credentials().await
    }

    async fn init_workspace(&self, path: PathBuf) -> anyhow::Result<WorkspaceId> {
        self.workspace_service().init_workspace(path).await
    }
}
