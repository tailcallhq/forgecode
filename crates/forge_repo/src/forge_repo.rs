use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use forge_app::{
    AgentRepository, CommandInfra, DirectoryReaderInfra, EnvironmentInfra, FileDirectoryInfra,
    FileInfoInfra, FileReaderInfra, FileRemoverInfra, FileWriterInfra, GrpcInfra, HttpInfra,
    KVStore, McpServerInfra, StrategyFactory, UserInfra, WalkedFile, Walker, WalkerInfra,
};
use forge_config::ForgeConfig;
use forge_domain::{
    AnyProvider, AuthCredential, ChatCompletionMessage, ChatRepository, CommandOutput, Context,
    Conversation, ConversationId, ConversationRepository, ConversationSummary, Environment,
    FileInfo, ForgeImportReport, FuzzySearchRepository, McpServerConfig, MigrationResult, Model,
    ModelId, Provider, ProviderId, ProviderRepository, ResultStream, SearchMatch, Skill,
    SkillRepository, Snapshot, SnapshotRepository, TextPatchBlock, TextPatchRepository,
};
use forge_eventsource::EventSource;
// Re-export CacacheStorage from forge_infra
pub use forge_infra::CacacheStorage;
use reqwest::Response;
use reqwest::header::HeaderMap;
use url::Url;

use crate::agent::ForgeAgentRepository;
use crate::context_engine::ForgeContextEngineRepository;
use crate::conversation::ConversationRepositoryImpl;
use crate::daemon_repo::DaemonConversationRepository;
use crate::database::{DatabasePool, PoolConfig};
use crate::fs_snap::ForgeFileSnapshotService;
use crate::fuzzy_search::ForgeFuzzySearchRepository;
use crate::provider::{ForgeChatRepository, ForgeProviderRepository};
use crate::skill::ForgeSkillRepository;
use crate::validation::ForgeValidationRepository;

/// Repository layer that implements all domain repository traits
///
/// This struct aggregates all repository implementations and provides a single
/// point of access for data persistence operations.
#[derive(Clone)]
pub struct ForgeRepo<F> {
    infra: Arc<F>,
    file_snapshot_service: Arc<ForgeFileSnapshotService>,
    conversation_repository: Arc<dyn ConversationRepository>,
    mcp_cache_repository: Arc<CacacheStorage>,
    provider_repository: Arc<ForgeProviderRepository<F>>,
    chat_repository: Arc<ForgeChatRepository<F>>,
    codebase_repo: Arc<ForgeContextEngineRepository<F>>,
    agent_repository: Arc<ForgeAgentRepository<F>>,
    skill_repository: Arc<ForgeSkillRepository<F>>,
    validation_repository: Arc<ForgeValidationRepository<F>>,
    fuzzy_search_repository: Arc<ForgeFuzzySearchRepository<F>>,
}

impl<
    F: EnvironmentInfra<Config = forge_config::ForgeConfig>
        + FileReaderInfra
        + FileWriterInfra
        + GrpcInfra
        + HttpInfra,
> ForgeRepo<F>
{
    pub fn new(infra: Arc<F>) -> Self {
        let env = infra.get_environment();
        let file_snapshot_service = Arc::new(ForgeFileSnapshotService::new(env.clone()));

        // Split-DB: primary DB is the write path; legacy DB is the historical
        // path, which the connection customizer ATTACHes read-only on every
        // acquire. When the two paths differ, the read-side `conversations_all`
        // TEMP VIEW exposes the UNION of both tables. When they are the same
        // (e.g. fresh install) the legacy attachment is a no-op.
        let write_path = env.write_database_path();
        let legacy_path = env.legacy_database_path();
        let legacy_for_pool = if legacy_path != write_path && legacy_path.exists() {
            Some(legacy_path.clone())
        } else {
            None
        };
        let pool_config = PoolConfig::new(write_path).with_legacy_database_path(legacy_for_pool);
        let db_pool = Arc::new(DatabasePool::try_from(pool_config).unwrap());
        let conversation_repository = Arc::new(ConversationRepositoryImpl::new(
            db_pool.clone(),
            env.workspace_hash(),
        ));
        // Daemon mode (P3 single-writer daemon): when FORGE_DBD_ENABLED, route
        // the hot-path conversation writes through forge-dbd and keep reads
        // direct. The decorator is best-effort — any daemon failure falls back
        // to the direct implementation, so this is a mode switch only.
        let conversation_repository: Arc<dyn ConversationRepository> = if env.dbd_enabled() {
            Arc::new(DaemonConversationRepository::new(
                conversation_repository,
                env.dbd_socket_path(),
                env.workspace_hash().id() as i64,
            ))
        } else {
            conversation_repository
        };

        let mcp_cache_repository = Arc::new(CacacheStorage::new(
            env.cache_dir().join("mcp_cache"),
            Some(3600),
        )); // 1 hour TTL

        let provider_repository = Arc::new(ForgeProviderRepository::new(infra.clone()));
        let chat_repository = Arc::new(ForgeChatRepository::new(infra.clone()));

        let codebase_repo = Arc::new(ForgeContextEngineRepository::new(infra.clone()));
        let agent_repository = Arc::new(ForgeAgentRepository::new(infra.clone()));
        let skill_repository = Arc::new(ForgeSkillRepository::new(infra.clone()));
        let validation_repository = Arc::new(ForgeValidationRepository::new(infra.clone()));
        let fuzzy_search_repository = Arc::new(ForgeFuzzySearchRepository::new(infra.clone()));
        Self {
            infra,
            file_snapshot_service,
            conversation_repository,
            mcp_cache_repository,
            provider_repository,
            chat_repository,
            codebase_repo,
            agent_repository,
            skill_repository,
            validation_repository,
            fuzzy_search_repository,
        }
    }
}

#[async_trait::async_trait]
impl<F: Send + Sync> SnapshotRepository for ForgeRepo<F> {
    async fn insert_snapshot(&self, file_path: &Path) -> anyhow::Result<Snapshot> {
        self.file_snapshot_service.insert_snapshot(file_path).await
    }

    async fn undo_snapshot(&self, file_path: &Path) -> anyhow::Result<()> {
        self.file_snapshot_service.undo_snapshot(file_path).await
    }
}

#[async_trait::async_trait]
impl<F: Send + Sync> ConversationRepository for ForgeRepo<F> {
    async fn upsert_conversation(&self, conversation: Conversation) -> anyhow::Result<()> {
        self.conversation_repository
            .upsert_conversation(conversation)
            .await
    }

    async fn get_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> anyhow::Result<Option<Conversation>> {
        self.conversation_repository
            .get_conversation(conversation_id)
            .await
    }

    async fn get_all_conversations(
        &self,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        self.conversation_repository
            .get_all_conversations(limit)
            .await
    }

    async fn get_last_conversation(&self) -> anyhow::Result<Option<Conversation>> {
        self.conversation_repository.get_last_conversation().await
    }

    async fn get_conversations_by_parent(
        &self,
        parent_id: &ConversationId,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        self.conversation_repository
            .get_conversations_by_parent(parent_id)
            .await
    }

    async fn get_parent_conversations(
        &self,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        self.conversation_repository
            .get_parent_conversations(limit)
            .await
    }

    async fn get_parent_conversations_lite(
        &self,
        limit: Option<usize>,
        all_workspaces: bool,
    ) -> anyhow::Result<Option<Vec<ConversationSummary>>> {
        self.conversation_repository
            .get_parent_conversations_lite(limit, all_workspaces)
            .await
    }

    async fn get_conversations_by_source(
        &self,
        source: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        self.conversation_repository
            .get_conversations_by_source(source, limit)
            .await
    }

    async fn delete_conversation(&self, conversation_id: &ConversationId) -> anyhow::Result<()> {
        self.conversation_repository
            .delete_conversation(conversation_id)
            .await
    }

    async fn upsert_conversation_ref(&self, conversation: &Conversation) -> anyhow::Result<()> {
        self.conversation_repository
            .upsert_conversation_ref(conversation)
            .await
    }

    async fn search_conversations(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<Conversation>> {
        self.conversation_repository
            .search_conversations(query, limit)
            .await
    }

    async fn optimize_fts_index(&self) -> anyhow::Result<()> {
        self.conversation_repository.optimize_fts_index().await
    }

    async fn refresh_fts_index(&self) -> anyhow::Result<()> {
        self.conversation_repository.refresh_fts_index().await
    }

    async fn update_parent_id(
        &self,
        conversation_id: &ConversationId,
        new_parent_id: Option<&ConversationId>,
    ) -> anyhow::Result<()> {
        self.conversation_repository
            .update_parent_id(conversation_id, new_parent_id)
            .await
    }

    async fn get_conversations_by_cwd(
        &self,
        cwd: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        self.conversation_repository
            .get_conversations_by_cwd(cwd, limit)
            .await
    }

    async fn get_conversation_snippet(
        &self,
        conversation_id: &ConversationId,
        query: &str,
        token_count: usize,
    ) -> anyhow::Result<Option<String>> {
        self.conversation_repository
            .get_conversation_snippet(conversation_id, query, token_count)
            .await
    }

    async fn mark_intent_state(
        &self,
        conversation_id: &ConversationId,
        new_state: &str,
    ) -> anyhow::Result<()> {
        self.conversation_repository
            .mark_intent_state(conversation_id, new_state)
            .await
    }

    async fn list_prune_eligible(
        &self,
        workspace_id: Option<i64>,
        limit: usize,
    ) -> anyhow::Result<Vec<Conversation>> {
        self.conversation_repository
            .list_prune_eligible(workspace_id, limit)
            .await
    }

    async fn prune_conversation(&self, conversation_id: &ConversationId) -> anyhow::Result<()> {
        self.conversation_repository
            .prune_conversation(conversation_id)
            .await
    }

    async fn rewind_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> anyhow::Result<Option<Conversation>> {
        self.conversation_repository
            .rewind_conversation(conversation_id)
            .await
    }

    async fn compress_uncompressed_contexts(&self) -> anyhow::Result<(usize, usize, usize)> {
        self.conversation_repository
            .compress_uncompressed_contexts()
            .await
    }

    async fn import_forge_db(&self, source: PathBuf) -> anyhow::Result<ForgeImportReport> {
        self.conversation_repository.import_forge_db(source).await
    }

    async fn import_forge_db_with_options(
        &self,
        source: PathBuf,
        options: &forge_domain::ForgeImportOptions,
    ) -> anyhow::Result<ForgeImportReport> {
        self.conversation_repository
            .import_forge_db_with_options(source, options)
            .await
    }

    async fn export_forge_db(
        &self,
        dest: PathBuf,
        options: &forge_domain::ForgeExportOptions,
    ) -> anyhow::Result<forge_domain::ForgeExportReport> {
        self.conversation_repository
            .export_forge_db(dest, options)
            .await
    }

    async fn database_stats(&self) -> anyhow::Result<forge_domain::HeliosdoctorDbStats> {
        self.conversation_repository.database_stats().await
    }

    async fn forget_conversations(
        &self,
        options: &forge_domain::ForgeForgetOptions,
    ) -> anyhow::Result<forge_domain::ForgeForgetReport> {
        self.conversation_repository
            .forget_conversations(options)
            .await
    }

    async fn migrate_data_dir(
        &self,
        options: &forge_domain::MigrateOptions,
    ) -> anyhow::Result<forge_domain::ForgeMigrateReport> {
        self.conversation_repository.migrate_data_dir(options).await
    }
}

#[async_trait::async_trait]
impl<
    F: EnvironmentInfra<Config = forge_config::ForgeConfig>
        + FileReaderInfra
        + FileWriterInfra
        + HttpInfra
        + Send
        + Sync,
> ChatRepository for ForgeRepo<F>
{
    async fn chat(
        &self,
        model_id: &ModelId,
        context: Context,
        provider: Provider<Url>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error> {
        self.chat_repository.chat(model_id, context, provider).await
    }

    async fn models(&self, provider: Provider<Url>) -> anyhow::Result<Vec<Model>> {
        self.chat_repository.models(provider).await
    }
}

#[async_trait::async_trait]
impl<
    F: EnvironmentInfra<Config = forge_config::ForgeConfig>
        + FileReaderInfra
        + FileWriterInfra
        + HttpInfra
        + Send
        + Sync,
> ProviderRepository for ForgeRepo<F>
{
    async fn get_all_providers(&self) -> anyhow::Result<Vec<AnyProvider>> {
        self.provider_repository.get_all_providers().await
    }

    async fn get_provider(&self, id: ProviderId) -> anyhow::Result<forge_domain::ProviderTemplate> {
        self.provider_repository.get_provider(id).await
    }

    async fn upsert_credential(&self, credential: AuthCredential) -> anyhow::Result<()> {
        // All providers now use file-based credentials
        self.provider_repository.upsert_credential(credential).await
    }

    async fn get_credential(&self, id: &ProviderId) -> anyhow::Result<Option<AuthCredential>> {
        self.provider_repository.get_credential(id).await
    }

    async fn remove_credential(&self, id: &ProviderId) -> anyhow::Result<()> {
        // All providers now use file-based credentials
        self.provider_repository.remove_credential(id).await
    }

    async fn migrate_env_credentials(&self) -> anyhow::Result<Option<MigrationResult>> {
        self.provider_repository.migrate_env_to_file().await
    }
}

#[async_trait::async_trait]
impl<F: EnvironmentInfra<Config = forge_config::ForgeConfig> + Send + Sync> EnvironmentInfra
    for ForgeRepo<F>
{
    type Config = forge_config::ForgeConfig;

    fn get_environment(&self) -> Environment {
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

    fn get_env_vars(&self) -> BTreeMap<String, String> {
        self.infra.get_env_vars()
    }

    fn database_stats(
        &self,
    ) -> impl std::future::Future<Output = anyhow::Result<forge_domain::HeliosdoctorDbStats>> + Send
    {
        self.infra.database_stats()
    }

    fn database_integrity(
        &self,
    ) -> impl std::future::Future<Output = anyhow::Result<forge_domain::HeliosdoctorDbStats>> + Send
    {
        self.infra.database_integrity()
    }
}

#[async_trait::async_trait]
impl<F: Send + Sync> KVStore for ForgeRepo<F> {
    async fn cache_get<K, V>(&self, key: &K) -> anyhow::Result<Option<V>>
    where
        K: std::hash::Hash + Sync,
        V: serde::Serialize + serde::de::DeserializeOwned + Send,
    {
        self.mcp_cache_repository.cache_get(key).await
    }

    async fn cache_set<K, V>(&self, key: &K, value: &V) -> anyhow::Result<()>
    where
        K: std::hash::Hash + Sync,
        V: serde::Serialize + Sync,
    {
        self.mcp_cache_repository.cache_set(key, value).await
    }

    async fn cache_clear(&self) -> anyhow::Result<()> {
        self.mcp_cache_repository.cache_clear().await
    }
}

#[async_trait::async_trait]
impl<F: HttpInfra> HttpInfra for ForgeRepo<F> {
    async fn http_get(&self, url: &Url, headers: Option<HeaderMap>) -> anyhow::Result<Response> {
        self.infra.http_get(url, headers).await
    }

    async fn http_post(
        &self,
        url: &Url,
        headers: Option<HeaderMap>,
        body: Bytes,
    ) -> anyhow::Result<Response> {
        self.infra.http_post(url, headers, body).await
    }

    async fn http_delete(&self, url: &Url) -> anyhow::Result<Response> {
        self.infra.http_delete(url).await
    }

    async fn http_eventsource(
        &self,
        url: &Url,
        headers: Option<HeaderMap>,
        body: Bytes,
    ) -> anyhow::Result<EventSource> {
        self.infra.http_eventsource(url, headers, body).await
    }
}

#[async_trait::async_trait]
impl<F> FileReaderInfra for ForgeRepo<F>
where
    F: FileReaderInfra + Send + Sync,
{
    async fn read_utf8(&self, path: &Path) -> anyhow::Result<String> {
        self.infra.read_utf8(path).await
    }

    fn read_batch_utf8(
        &self,
        batch_size: usize,
        paths: Vec<PathBuf>,
    ) -> impl futures::Stream<Item = (PathBuf, anyhow::Result<String>)> + Send {
        self.infra.read_batch_utf8(batch_size, paths)
    }

    async fn read(&self, path: &Path) -> anyhow::Result<Vec<u8>> {
        self.infra.read(path).await
    }

    async fn range_read_utf8(
        &self,
        path: &Path,
        start_line: u64,
        end_line: u64,
    ) -> anyhow::Result<(String, FileInfo)> {
        self.infra.range_read_utf8(path, start_line, end_line).await
    }
}

#[async_trait::async_trait]
impl<F> WalkerInfra for ForgeRepo<F>
where
    F: WalkerInfra + Send + Sync,
{
    async fn walk(&self, config: Walker) -> anyhow::Result<Vec<WalkedFile>> {
        self.infra.walk(config).await
    }
}

#[async_trait::async_trait]
impl<F> FileWriterInfra for ForgeRepo<F>
where
    F: FileWriterInfra + Send + Sync,
{
    async fn write(&self, path: &Path, contents: Bytes) -> anyhow::Result<()> {
        self.infra.write(path, contents).await
    }
    async fn append(&self, path: &Path, contents: Bytes) -> anyhow::Result<()> {
        self.infra.append(path, contents).await
    }
    async fn write_temp(&self, prefix: &str, ext: &str, content: &str) -> anyhow::Result<PathBuf> {
        self.infra.write_temp(prefix, ext, content).await
    }
}

#[async_trait::async_trait]
impl<F> FileInfoInfra for ForgeRepo<F>
where
    F: FileInfoInfra + Send + Sync,
{
    async fn is_binary(&self, path: &Path) -> anyhow::Result<bool> {
        self.infra.is_binary(path).await
    }
    async fn is_file(&self, path: &Path) -> anyhow::Result<bool> {
        self.infra.is_file(path).await
    }
    async fn exists(&self, path: &Path) -> anyhow::Result<bool> {
        self.infra.exists(path).await
    }
    async fn file_size(&self, path: &Path) -> anyhow::Result<u64> {
        self.infra.file_size(path).await
    }
}

#[async_trait::async_trait]
impl<F> FileDirectoryInfra for ForgeRepo<F>
where
    F: FileDirectoryInfra + Send + Sync,
{
    async fn create_dirs(&self, path: &Path) -> anyhow::Result<()> {
        self.infra.create_dirs(path).await
    }
}

#[async_trait::async_trait]
impl<F> FileRemoverInfra for ForgeRepo<F>
where
    F: FileRemoverInfra + Send + Sync,
{
    async fn remove(&self, path: &Path) -> anyhow::Result<()> {
        self.infra.remove(path).await
    }
}

#[async_trait::async_trait]
impl<F> DirectoryReaderInfra for ForgeRepo<F>
where
    F: DirectoryReaderInfra + Send + Sync,
{
    async fn list_directory_entries(
        &self,
        directory: &Path,
    ) -> anyhow::Result<Vec<(PathBuf, bool)>> {
        self.infra.list_directory_entries(directory).await
    }

    async fn read_directory_files(
        &self,
        directory: &Path,
        pattern: Option<&str>, // Optional glob pattern like "*.md"
    ) -> anyhow::Result<Vec<(PathBuf, String)>> {
        self.infra.read_directory_files(directory, pattern).await
    }
}

#[async_trait::async_trait]
impl<F> UserInfra for ForgeRepo<F>
where
    F: UserInfra + Send + Sync,
{
    async fn prompt_question(&self, question: &str) -> anyhow::Result<Option<String>> {
        self.infra.prompt_question(question).await
    }

    async fn select_one<T: Clone + std::fmt::Display + Send + 'static>(
        &self,
        message: &str,
        options: Vec<T>,
    ) -> anyhow::Result<Option<T>> {
        self.infra.select_one(message, options).await
    }

    async fn select_one_enum<T>(&self, message: &str) -> anyhow::Result<Option<T>>
    where
        T: Clone + std::fmt::Display + Send + 'static + strum::IntoEnumIterator + std::str::FromStr,
        <T as std::str::FromStr>::Err: std::fmt::Debug,
    {
        self.infra.select_one_enum(message).await
    }

    async fn select_many<T: std::fmt::Display + Clone + Send + 'static>(
        &self,
        message: &str,
        options: Vec<T>,
    ) -> anyhow::Result<Option<Vec<T>>> {
        self.infra.select_many(message, options).await
    }
}

#[async_trait::async_trait]
impl<F> McpServerInfra for ForgeRepo<F>
where
    F: McpServerInfra + Send + Sync,
{
    type Client = F::Client;

    async fn connect(
        &self,
        config: McpServerConfig,
        env_vars: &BTreeMap<String, String>,
        environment: &Environment,
    ) -> anyhow::Result<F::Client> {
        self.infra.connect(config, env_vars, environment).await
    }
}

#[async_trait::async_trait]
impl<F> CommandInfra for ForgeRepo<F>
where
    F: CommandInfra + Send + Sync,
{
    async fn execute_command(
        &self,
        command: String,
        working_dir: PathBuf,
        silent: bool,
        env_vars: Option<Vec<String>>,
    ) -> anyhow::Result<CommandOutput> {
        self.infra
            .execute_command(command, working_dir, silent, env_vars)
            .await
    }

    async fn execute_command_raw(
        &self,
        command: &str,
        working_dir: PathBuf,
        env_vars: Option<Vec<String>>,
    ) -> anyhow::Result<std::process::ExitStatus> {
        self.infra
            .execute_command_raw(command, working_dir, env_vars)
            .await
    }
}

#[async_trait::async_trait]
impl<F: FileInfoInfra + EnvironmentInfra<Config = ForgeConfig> + DirectoryReaderInfra + Send + Sync>
    AgentRepository for ForgeRepo<F>
{
    async fn get_agents(&self) -> anyhow::Result<Vec<forge_domain::Agent>> {
        self.agent_repository.get_agents().await
    }

    async fn get_agent_infos(&self) -> anyhow::Result<Vec<forge_domain::AgentInfo>> {
        self.agent_repository.get_agent_infos().await
    }
}

#[async_trait::async_trait]
impl<F: FileInfoInfra + EnvironmentInfra + FileReaderInfra + WalkerInfra + Send + Sync>
    SkillRepository for ForgeRepo<F>
{
    async fn load_skills(&self) -> anyhow::Result<Vec<Skill>> {
        self.skill_repository.load_skills().await
    }
}

impl<F: StrategyFactory> StrategyFactory for ForgeRepo<F> {
    type Strategy = F::Strategy;

    fn create_auth_strategy(
        &self,
        provider_id: ProviderId,
        auth_method: forge_domain::AuthMethod,
        required_params: Vec<forge_domain::URLParamSpec>,
    ) -> anyhow::Result<Self::Strategy> {
        self.infra
            .create_auth_strategy(provider_id, auth_method, required_params)
    }
}

#[async_trait::async_trait]
impl<F: GrpcInfra + Send + Sync> forge_domain::WorkspaceIndexRepository for ForgeRepo<F> {
    async fn authenticate(&self) -> anyhow::Result<forge_domain::WorkspaceAuth> {
        self.codebase_repo.authenticate().await
    }

    async fn create_workspace(
        &self,
        working_dir: &std::path::Path,
        auth_token: &forge_domain::ApiKey,
    ) -> anyhow::Result<forge_domain::WorkspaceId> {
        self.codebase_repo
            .create_workspace(working_dir, auth_token)
            .await
    }

    async fn upload_files(
        &self,
        upload: &forge_domain::FileUpload,
        auth_token: &forge_domain::ApiKey,
    ) -> anyhow::Result<forge_domain::FileUploadInfo> {
        self.codebase_repo.upload_files(upload, auth_token).await
    }

    async fn search(
        &self,
        query: &forge_domain::CodeSearchQuery<'_>,
        auth_token: &forge_domain::ApiKey,
    ) -> anyhow::Result<Vec<forge_domain::Node>> {
        self.codebase_repo.search(query, auth_token).await
    }

    async fn list_workspaces(
        &self,
        auth_token: &forge_domain::ApiKey,
    ) -> anyhow::Result<Vec<forge_domain::WorkspaceInfo>> {
        self.codebase_repo.list_workspaces(auth_token).await
    }

    async fn get_workspace(
        &self,
        workspace_id: &forge_domain::WorkspaceId,
        auth_token: &forge_domain::ApiKey,
    ) -> anyhow::Result<Option<forge_domain::WorkspaceInfo>> {
        self.codebase_repo
            .get_workspace(workspace_id, auth_token)
            .await
    }

    async fn list_workspace_files(
        &self,
        workspace: &forge_domain::WorkspaceFiles,
        auth_token: &forge_domain::ApiKey,
    ) -> anyhow::Result<Vec<forge_domain::FileHash>> {
        self.codebase_repo
            .list_workspace_files(workspace, auth_token)
            .await
    }

    async fn delete_files(
        &self,
        deletion: &forge_domain::FileDeletion,
        auth_token: &forge_domain::ApiKey,
    ) -> anyhow::Result<()> {
        self.codebase_repo.delete_files(deletion, auth_token).await
    }

    async fn delete_workspace(
        &self,
        workspace_id: &forge_domain::WorkspaceId,
        auth_token: &forge_domain::ApiKey,
    ) -> anyhow::Result<()> {
        self.codebase_repo
            .delete_workspace(workspace_id, auth_token)
            .await
    }
}

#[async_trait::async_trait]
impl<F: GrpcInfra + Send + Sync> forge_domain::ValidationRepository for ForgeRepo<F> {
    async fn validate_file(
        &self,
        path: impl AsRef<std::path::Path> + Send,
        content: &str,
    ) -> anyhow::Result<Vec<forge_domain::SyntaxError>> {
        self.validation_repository
            .validate_file(path, content)
            .await
    }
}

#[async_trait::async_trait]
impl<F: GrpcInfra + Send + Sync> FuzzySearchRepository for ForgeRepo<F> {
    async fn fuzzy_search(
        &self,
        needle: &str,
        haystack: &str,
        search_all: bool,
    ) -> anyhow::Result<Vec<SearchMatch>> {
        self.fuzzy_search_repository
            .fuzzy_search(needle, haystack, search_all)
            .await
    }
}

#[async_trait::async_trait]
impl<F: GrpcInfra + Send + Sync> TextPatchRepository for ForgeRepo<F> {
    async fn build_text_patch(
        &self,
        haystack: &str,
        old_string: &str,
        new_string: &str,
    ) -> anyhow::Result<TextPatchBlock> {
        let request = tonic::Request::new(crate::proto_generated::BuildTextPatchRequest {
            haystack: haystack.to_string(),
            old_string: old_string.to_string(),
            new_string: new_string.to_string(),
        });

        let channel = self.infra.channel()?;
        let mut client =
            crate::proto_generated::forge_service_client::ForgeServiceClient::new(channel);
        let response = client.build_text_patch(request).await?.into_inner();

        Ok(TextPatchBlock { patch: response.patch, patched_text: response.patched_text })
    }
}

impl<F: GrpcInfra> GrpcInfra for ForgeRepo<F> {
    fn channel(&self) -> anyhow::Result<tonic::transport::Channel> {
        self.infra.channel()
    }

    fn hydrate(&self) {
        self.infra.hydrate();
    }
}

impl<F: forge_domain::ConsoleWriter> forge_domain::ConsoleWriter for ForgeRepo<F> {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use forge_app::{EnvironmentInfra, FileReaderInfra, FileWriterInfra, GrpcInfra, HttpInfra};
    use forge_config::ForgeConfig;
    use forge_domain::{Environment, HeliosdoctorDbStats};

    use super::*;

    /// Minimal infra double that reports distinguishable `database_stats` and
    /// `database_integrity` results so a forwarding regression (falling back
    /// to the full-stats default) is observable.
    struct MockInfra {
        base_path: PathBuf,
    }

    #[allow(clippy::manual_async_fn)] // The trait intentionally uses RPIT futures; keep the mock signature identical.
    impl EnvironmentInfra for MockInfra {
        type Config = ForgeConfig;

        fn get_env_var(&self, _key: &str) -> Option<String> {
            None
        }

        fn get_env_vars(&self) -> BTreeMap<String, String> {
            BTreeMap::new()
        }

        fn get_environment(&self) -> Environment {
            Environment {
                os: "test".to_string(),
                cwd: self.base_path.clone(),
                home: None,
                shell: "bash".to_string(),
                base_path: self.base_path.clone(),
            }
        }

        fn get_config(&self) -> anyhow::Result<Self::Config> {
            Ok(ForgeConfig::default())
        }

        fn update_environment(
            &self,
            _ops: Vec<forge_domain::ConfigOperation>,
        ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send {
            async move { Ok(()) }
        }

        fn database_stats(
            &self,
        ) -> impl std::future::Future<Output = anyhow::Result<HeliosdoctorDbStats>> + Send {
            async move { Ok(HeliosdoctorDbStats { total_conversations: 42, ..Default::default() }) }
        }

        fn database_integrity(
            &self,
        ) -> impl std::future::Future<Output = anyhow::Result<HeliosdoctorDbStats>> + Send {
            async move {
                Ok(HeliosdoctorDbStats { integrity_check: "ok".to_string(), ..Default::default() })
            }
        }
    }

    #[async_trait::async_trait]
    impl FileReaderInfra for MockInfra {
        async fn read_utf8(&self, _path: &Path) -> anyhow::Result<String> {
            Ok(String::new())
        }

        fn read_batch_utf8(
            &self,
            _batch_size: usize,
            _paths: Vec<PathBuf>,
        ) -> impl futures::Stream<Item = (PathBuf, anyhow::Result<String>)> + Send {
            futures::stream::empty()
        }

        async fn read(&self, _path: &Path) -> anyhow::Result<Vec<u8>> {
            Ok(Vec::new())
        }

        async fn range_read_utf8(
            &self,
            _path: &Path,
            _start_line: u64,
            _end_line: u64,
        ) -> anyhow::Result<(String, FileInfo)> {
            anyhow::bail!("not implemented in mock")
        }
    }

    #[async_trait::async_trait]
    impl FileWriterInfra for MockInfra {
        async fn write(&self, _path: &Path, _contents: Bytes) -> anyhow::Result<()> {
            Ok(())
        }

        async fn append(&self, _path: &Path, _contents: Bytes) -> anyhow::Result<()> {
            Ok(())
        }

        async fn write_temp(
            &self,
            _prefix: &str,
            _ext: &str,
            _content: &str,
        ) -> anyhow::Result<PathBuf> {
            anyhow::bail!("not implemented in mock")
        }
    }

    impl GrpcInfra for MockInfra {
        fn channel(&self) -> anyhow::Result<tonic::transport::Channel> {
            anyhow::bail!("not implemented in mock")
        }

        fn hydrate(&self) {}
    }

    #[async_trait::async_trait]
    impl HttpInfra for MockInfra {
        async fn http_get(
            &self,
            _url: &Url,
            _headers: Option<HeaderMap>,
        ) -> anyhow::Result<Response> {
            anyhow::bail!("not implemented in mock")
        }

        async fn http_post(
            &self,
            _url: &Url,
            _headers: Option<HeaderMap>,
            _body: bytes::Bytes,
        ) -> anyhow::Result<Response> {
            anyhow::bail!("not implemented in mock")
        }

        async fn http_delete(&self, _url: &Url) -> anyhow::Result<Response> {
            anyhow::bail!("not implemented in mock")
        }

        async fn http_eventsource(
            &self,
            _url: &Url,
            _headers: Option<HeaderMap>,
            _body: Bytes,
        ) -> anyhow::Result<EventSource> {
            anyhow::bail!("not implemented in mock")
        }
    }

    #[tokio::test]
    async fn environment_infra_database_integrity_forwards_to_inner_infra() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let infra = Arc::new(MockInfra { base_path: tmp.path().to_path_buf() });
        let repo = ForgeRepo::new(infra.clone());

        // The full-stats fallback would surface total_conversations=42;
        // the PRAGMA-only forward must surface the integrity marker instead.
        let stats = forge_app::EnvironmentInfra::database_integrity(&repo).await?;
        assert_eq!(stats.integrity_check, "ok");
        assert_eq!(stats.total_conversations, 0);

        // And the stats path must still forward to the inner stats impl.
        let stats = forge_app::EnvironmentInfra::database_stats(&repo).await?;
        assert_eq!(stats.total_conversations, 42);
        Ok(())
    }
}
