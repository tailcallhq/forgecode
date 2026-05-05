use std::path::Path;

use anyhow::Result;
use url::Url;

use crate::{
    AnyProvider, AuthCredential, ChatCompletionMessage, Context, Conversation, ConversationId,
    MigrationResult, Model, ModelId, Provider, ProviderId, ProviderTemplate, ResultStream,
    SearchMatch, Skill, Snapshot, WorkspaceAuth, WorkspaceId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPatchBlock {
    pub patch: String,
    pub patched_text: String,
}

/// Repository for managing file snapshots
///
/// This repository provides operations for creating and restoring file
/// snapshots, enabling undo functionality for file modifications.
#[async_trait::async_trait]
pub trait SnapshotRepository: Send + Sync {
    /// Inserts a new snapshot for the given file path
    ///
    /// # Arguments
    /// * `file_path` - Path to the file to snapshot
    ///
    /// # Errors
    /// Returns an error if the snapshot creation fails
    async fn insert_snapshot(&self, file_path: &Path) -> Result<Snapshot>;

    /// Restores the most recent snapshot for the given file path
    ///
    /// # Arguments
    /// * `file_path` - Path to the file to restore
    ///
    /// # Errors
    /// Returns an error if no snapshot exists or restoration fails
    async fn undo_snapshot(&self, file_path: &Path) -> Result<()>;
}

/// Repository for managing conversation persistence
///
/// This repository provides CRUD operations for conversations, including
/// creating, retrieving, and listing conversations.
#[async_trait::async_trait]
pub trait ConversationRepository: Send + Sync {
    /// Creates or updates a conversation
    ///
    /// # Arguments
    /// * `conversation` - The conversation to persist
    ///
    /// # Errors
    /// Returns an error if the operation fails
    async fn upsert_conversation(&self, conversation: Conversation) -> Result<()>;

    /// Retrieves a conversation by its ID
    ///
    /// # Arguments
    /// * `conversation_id` - The ID of the conversation to retrieve
    ///
    /// # Errors
    /// Returns an error if the operation fails
    async fn get_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<Conversation>>;

    /// Retrieves all conversations with an optional limit
    ///
    /// # Arguments
    /// * `limit` - Optional maximum number of conversations to retrieve
    ///
    /// # Errors
    /// Returns an error if the operation fails
    async fn get_all_conversations(
        &self,
        limit: Option<usize>,
    ) -> Result<Option<Vec<Conversation>>>;

    /// Retrieves the most recent conversation
    ///
    /// # Errors
    /// Returns an error if the operation fails
    async fn get_last_conversation(&self) -> Result<Option<Conversation>>;

    /// Permanently deletes a conversation
    ///
    /// # Arguments
    /// * `conversation_id` - The ID of the conversation to delete
    ///
    /// # Errors
    /// Returns an error if the operation fails
    async fn delete_conversation(&self, conversation_id: &ConversationId) -> Result<()>;
}

#[async_trait::async_trait]
pub trait ChatRepository: Send + Sync {
    async fn chat(
        &self,
        model_id: &ModelId,
        context: Context,
        provider: Provider<Url>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error>;
    async fn models(&self, provider: Provider<Url>) -> anyhow::Result<Vec<Model>>;
}

#[async_trait::async_trait]
pub trait ProviderRepository: Send + Sync {
    async fn get_all_providers(&self) -> anyhow::Result<Vec<AnyProvider>>;
    async fn get_provider(&self, id: ProviderId) -> anyhow::Result<ProviderTemplate>;
    async fn upsert_credential(&self, credential: AuthCredential) -> anyhow::Result<()>;
    async fn get_credential(&self, id: &ProviderId) -> anyhow::Result<Option<AuthCredential>>;
    async fn remove_credential(&self, id: &ProviderId) -> anyhow::Result<()>;
    async fn migrate_env_credentials(&self) -> anyhow::Result<Option<MigrationResult>>;
}

/// Repository for managing workspace indexing and search operations
#[async_trait::async_trait]
pub trait WorkspaceIndexRepository: Send + Sync {
    /// Authenticate with the indexing service via gRPC API
    async fn authenticate(&self) -> anyhow::Result<WorkspaceAuth>;

    /// Create a new workspace on the indexing server
    async fn create_workspace(
        &self,
        working_dir: &std::path::Path,
        auth_token: &crate::ApiKey,
    ) -> anyhow::Result<WorkspaceId>;

    /// Upload files to be indexed
    async fn upload_files(
        &self,
        upload: &crate::FileUpload,
        auth_token: &crate::ApiKey,
    ) -> anyhow::Result<crate::FileUploadInfo>;

    /// Search the indexed codebase using semantic search
    async fn search(
        &self,
        query: &crate::CodeSearchQuery<'_>,
        auth_token: &crate::ApiKey,
    ) -> anyhow::Result<Vec<crate::Node>>;

    /// List all workspaces for a user
    async fn list_workspaces(
        &self,
        auth_token: &crate::ApiKey,
    ) -> anyhow::Result<Vec<crate::WorkspaceInfo>>;

    /// Get workspace information by workspace ID
    async fn get_workspace(
        &self,
        workspace_id: &WorkspaceId,
        auth_token: &crate::ApiKey,
    ) -> anyhow::Result<Option<crate::WorkspaceInfo>>;

    /// List all files in a workspace with their hashes
    async fn list_workspace_files(
        &self,
        workspace: &crate::WorkspaceFiles,
        auth_token: &crate::ApiKey,
    ) -> anyhow::Result<Vec<crate::FileHash>>;

    /// Delete files from a workspace
    async fn delete_files(
        &self,
        deletion: &crate::FileDeletion,
        auth_token: &crate::ApiKey,
    ) -> anyhow::Result<()>;

    /// Delete a workspace and all its indexed data
    async fn delete_workspace(
        &self,
        workspace_id: &WorkspaceId,
        auth_token: &crate::ApiKey,
    ) -> anyhow::Result<()>;
}

/// Repository for managing skills
///
/// This repository provides operations for loading and managing skills from
/// markdown files.
#[async_trait::async_trait]
pub trait SkillRepository: Send + Sync {
    /// Loads all available skills from the skills directory
    ///
    /// # Errors
    /// Returns an error if skill loading fails
    async fn load_skills(&self) -> Result<Vec<Skill>>;
}

/// Repository for validating file syntax
///
/// This repository provides operations for validating the syntax of source
/// code files using remote validation services.
#[async_trait::async_trait]
pub trait ValidationRepository: Send + Sync {
    /// Validates the syntax of a single file
    ///
    /// # Arguments
    /// * `path` - Path to the file (used for determining language and in error
    ///   messages)
    /// * `content` - Content of the file to validate
    ///
    /// # Returns
    /// * `Ok(vec![])` - File is valid or file type is not supported by backend
    /// * `Ok(errors)` - Validation failed with list of syntax errors
    /// * `Err(_)` - Communication error with validation service
    async fn validate_file(
        &self,
        path: impl AsRef<std::path::Path> + Send,
        content: &str,
    ) -> Result<Vec<crate::SyntaxError>>;
}

/// Repository for fuzzy searching text
///
/// This repository provides fuzzy search functionality for searching
/// needle in haystack with optional search_all flag.
#[async_trait::async_trait]
pub trait FuzzySearchRepository: Send + Sync {
    /// Performs a fuzzy search for a needle in a haystack
    ///
    /// # Arguments
    /// * `needle` - The string to search for
    /// * `haystack` - The text to search in
    /// * `search_all` - Whether to search all matches or just the first
    ///
    /// # Returns
    /// * `Ok(Vec<SearchMatch>)` - List of matches with line ranges
    /// * `Err(_)` - Communication error with search service
    async fn fuzzy_search(
        &self,
        needle: &str,
        haystack: &str,
        search_all: bool,
    ) -> Result<Vec<SearchMatch>>;
}

#[async_trait::async_trait]
pub trait TextPatchRepository: Send + Sync {
    async fn build_text_patch(
        &self,
        haystack: &str,
        old_string: &str,
        new_string: &str,
    ) -> Result<TextPatchBlock>;
}
