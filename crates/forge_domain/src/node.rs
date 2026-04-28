use derive_more::Display;
use derive_setters::Setters;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::WorkspaceId;

/// Progress events emitted during workspace indexing
#[derive(Debug, Clone, PartialEq)]
pub enum SyncProgress {
    /// Sync operation is starting
    Starting,
    /// A new workspace was created on the server
    WorkspaceCreated {
        /// The ID of the newly created workspace
        workspace_id: WorkspaceId,
    },
    /// Discovering files in the directory
    DiscoveringFiles {
        /// ID of the current workspace
        workspace_id: WorkspaceId,
        /// Path being scanned
        path: std::path::PathBuf,
    },
    /// Files have been discovered in the directory
    FilesDiscovered {
        /// Total number of files found
        count: usize,
    },
    /// Comparing local files with server state
    ComparingFiles {
        /// Number of remote files in the workspace
        remote_files: usize,
        /// Number of local files being compared
        local_files: usize,
    },
    /// Diff computed showing breakdown of changes
    DiffComputed {
        /// Number of files added (new files)
        added: usize,
        /// Number of files deleted (orphaned on server)
        deleted: usize,
        /// Number of files modified (changed files)
        modified: usize,
    },
    /// Syncing files (deleting outdated + uploading new/changed)
    Syncing {
        /// Current progress
        current: usize,
        /// Total number of files to sync
        total: usize,
    },
    /// Sync operation completed successfully
    Completed {
        /// Total number of files in the workspace
        total_files: usize,
        /// Number of files that were uploaded (changed or new)
        uploaded_files: usize,
        /// Number of files that failed to sync
        failed_files: usize,
    },
}

impl SyncProgress {
    /// Returns the progress weight (0-100) for this event.
    pub fn weight(&self) -> Option<u64> {
        match self {
            Self::Syncing { current, total } => {
                let sync_progress = if *total > 0 {
                    (*current as f64) / (*total as f64) * 100.0
                } else {
                    0.0
                };
                Some(sync_progress as u64)
            }
            _ => None,
        }
    }
}

/// Stored authentication token for the indexing service (no expiry)
///
/// Associates a user with their indexing service authentication token
/// obtained from the remote authentication API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceAuth {
    /// User ID that owns this authentication
    pub user_id: UserId,
    /// Authentication token (obtained from HTTP API)
    pub token: crate::ApiKey,
    /// When this token was stored locally
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<WorkspaceAuth> for crate::AuthDetails {
    fn from(auth: WorkspaceAuth) -> Self {
        crate::AuthDetails::static_api_key(auth.token)
    }
}

impl WorkspaceAuth {
    /// Create a new indexing auth record
    pub fn new(user_id: UserId, token: crate::ApiKey) -> Self {
        Self { user_id, token, created_at: chrono::Utc::now() }
    }
}

/// File content for upload to workspace server
///
/// Contains the file path (relative to workspace root) and its textual content
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRead {
    /// File path (relative to workspace root)
    pub path: String,
    /// File content as UTF-8 text
    pub content: String,
}

impl FileRead {
    /// Create a new file read entry
    pub fn new(path: String, content: String) -> Self {
        Self { path, content }
    }
}

/// Generic wrapper for workspace operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBase<T> {
    pub user_id: UserId,
    pub workspace_id: WorkspaceId,
    pub data: T,
}

impl<T> CodeBase<T> {
    pub fn new(user_id: UserId, workspace_id: WorkspaceId, data: T) -> Self {
        Self { user_id, workspace_id, data }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Setters)]
#[setters(strip_option, into)]
pub struct SearchParams<'a> {
    pub query: &'a str,
    pub limit: Option<usize>,
    pub top_k: Option<u32>,
    pub use_case: String,
    pub starts_with: Option<String>,
    pub ends_with: Option<Vec<String>>,
}

impl<'a> SearchParams<'a> {
    pub fn new(query: &'a str, use_case: &str) -> Self {
        Self {
            query,
            limit: None,
            top_k: None,
            use_case: use_case.to_string(),
            starts_with: None,
            ends_with: None,
        }
    }
}

pub type CodeSearchQuery<'a> = CodeBase<SearchParams<'a>>;
pub type FileUpload = CodeBase<Vec<FileRead>>;
pub type FileDeletion = CodeBase<Vec<String>>;
pub type WorkspaceFiles = CodeBase<()>;

/// User identifier for codebase operations.
///
/// Unique per machine, generated once and stored in database.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Display)]
#[display("{}", _0)]
pub struct UserId(Uuid);

impl UserId {
    /// Generate a new random user ID
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    /// Parse a user ID from a string
    ///
    /// # Errors
    /// Returns an error if the string is not a valid UUID
    pub fn from_string(s: &str) -> anyhow::Result<Self> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Node identifier for code graph nodes.
///
/// Uniquely identifies a node in the codebase graph (file chunks, files,
/// notes, tasks, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Display)]
#[display("{}", _0)]
pub struct NodeId(String);

impl NodeId {
    /// Create a new node ID from a string
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the node ID as a string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for NodeId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for NodeId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl AsRef<str> for NodeId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Git repository information for a workspace
///
/// Contains commit hash and branch name for version tracking
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitInfo {
    /// Git commit hash (e.g., "abc123...")
    pub commit: String,
    /// Git branch name (e.g., "main", "develop")
    pub branch: String,
}

/// Information about a workspace from the server
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    /// Workspace ID
    pub workspace_id: WorkspaceId,
    /// Working directory path
    pub working_dir: String,
    /// Number of nodes created
    pub node_count: Option<u64>,
    /// Number of relations between nodes
    pub relation_count: Option<u64>,
    /// Last updated timestamp
    pub last_updated: Option<chrono::DateTime<chrono::Utc>>,
    /// Workspace created time.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Result of a codebase sync operation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Setters)]
pub struct FileUploadResponse {
    /// Workspace ID that was synced
    pub workspace_id: WorkspaceId,
    /// Number of files processed
    pub files_processed: usize,
    /// Upload statistics
    pub upload_stats: FileUploadInfo,
    /// Whether a new workspace was created (vs using existing)
    pub is_new_workspace: bool,
}

impl FileUploadResponse {
    /// Create new sync statistics
    pub fn new(
        workspace_id: WorkspaceId,
        files_processed: usize,
        upload_stats: FileUploadInfo,
    ) -> Self {
        Self {
            workspace_id,
            files_processed,
            upload_stats,
            is_new_workspace: false,
        }
    }
}

/// Statistics from uploading files to the codebase server
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FileUploadInfo {
    /// Number of code nodes created
    pub nodes_created: usize,
    /// Number of relations created
    pub relations_created: usize,
}

impl std::ops::Add for FileUploadInfo {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            nodes_created: self.nodes_created + other.nodes_created,
            relations_created: self.relations_created + other.relations_created,
        }
    }
}

impl FileUploadInfo {
    /// Create new upload statistics
    pub fn new(nodes_created: usize, relations_created: usize) -> Self {
        Self { nodes_created, relations_created }
    }
}

/// Results for a single codebase search query
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CodebaseQueryResult {
    /// The query string that was executed
    pub query: String,
    /// Relevance query used for re-ranking
    pub use_case: String,
    /// The search results for this query
    pub results: Vec<Node>,
}

/// Results for multiple codebase search queries
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CodebaseSearchResults {
    /// Results for each query/use_case pair
    pub queries: Vec<CodebaseQueryResult>,
}

/// A search result with its similarity score
///
/// Wraps a code node with its semantic search scores,
/// keeping the scores separate from the node data itself.
#[derive(
    Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, derive_setters::Setters,
)]
#[setters(strip_option)]
pub struct Node {
    /// Node identifier
    pub node_id: NodeId,
    /// The node data (file, chunk, note, etc.)
    #[serde(flatten)]
    pub node: NodeData,
    /// Relevance score (most important ranking metric)
    pub relevance: Option<f32>,
    /// Distance score (second ranking metric, lower is better)
    pub distance: Option<f32>,
}

/// File chunk with precise line numbers
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FileChunk {
    /// File path
    pub file_path: String,
    /// Code content
    pub content: String,
    /// Start line in the file
    pub start_line: u32,
    /// End line in the file
    pub end_line: u32,
}

/// Full file content
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FileNode {
    /// File path
    pub file_path: String,
    /// File content
    pub content: String,
    /// SHA-256 hash of the file content
    pub hash: String,
}

/// File reference (path only, no content)
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FileRef {
    /// File path
    pub file_path: String,
    /// SHA-256 hash of the file content
    pub file_hash: String,
}

/// Note content
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Note {
    /// Note content
    pub content: String,
}

/// Task description
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Task {
    /// Task description
    pub task: String,
}

/// Result of a semantic search query
///
/// Represents different types of nodes returned from the codebase service.
/// Each variant contains only the fields relevant to that node type.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, derive_more::From)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeData {
    /// File chunk with precise line numbers
    #[from]
    FileChunk(FileChunk),
    /// Full file content
    #[from]
    File(FileNode),
    /// File reference (path only, no content)
    #[from]
    FileRef(FileRef),
    /// Note content
    #[from]
    Note(Note),
    /// Task description
    #[from]
    Task(Task),
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_user_id_roundtrip() {
        let user_id = UserId::generate();
        let s = user_id.to_string();
        let parsed = UserId::from_string(&s).unwrap();
        assert_eq!(user_id, parsed);
    }

    #[test]
    fn test_workspace_id_roundtrip() {
        let workspace_id = WorkspaceId::generate();
        let s = workspace_id.to_string();
        let parsed = WorkspaceId::from_string(&s).unwrap();
        assert_eq!(workspace_id, parsed);
    }

    #[test]
    fn test_search_params_with_file_extension() {
        let actual = SearchParams::new("retry mechanism", "find retry logic")
            .limit(10usize)
            .top_k(20u32)
            .ends_with(vec![".rs".to_string()]);

        let expected = SearchParams {
            query: "retry mechanism",
            limit: Some(10),
            top_k: Some(20),
            use_case: "find retry logic".to_string(),
            starts_with: None,
            ends_with: Some(vec![".rs".to_string()]),
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_search_params_with_multiple_file_extensions() {
        let actual = SearchParams::new("retry mechanism", "find retry logic")
            .limit(10usize)
            .top_k(20u32)
            .ends_with(vec![
                ".rs".to_string(),
                ".ts".to_string(),
                ".py".to_string(),
            ]);

        let expected = SearchParams {
            query: "retry mechanism",
            limit: Some(10),
            top_k: Some(20),
            use_case: "find retry logic".to_string(),
            starts_with: None,
            ends_with: Some(vec![
                ".rs".to_string(),
                ".ts".to_string(),
                ".py".to_string(),
            ]),
        };

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_search_params_without_file_extension() {
        let actual = SearchParams::new("auth logic", "authentication implementation").limit(5usize);

        let expected = SearchParams {
            query: "auth logic",
            limit: Some(5),
            top_k: None,
            use_case: "authentication implementation".to_string(),
            starts_with: None,
            ends_with: None,
        };

        assert_eq!(actual, expected);
    }
}
