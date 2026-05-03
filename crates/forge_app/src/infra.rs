use std::collections::BTreeMap;
use std::hash::Hash;
use std::path::{Path, PathBuf};

use anyhow::Result;
use bytes::Bytes;
use forge_domain::{
    AuthCodeParams, CommandOutput, ConfigOperation, Environment, FileInfo, McpServerConfig,
    OAuthConfig, OAuthTokenResponse, ToolDefinition, ToolName, ToolOutput,
};
use forge_eventsource::EventSource;
use reqwest::Response;
use reqwest::header::HeaderMap;
use serde::de::DeserializeOwned;
use url::Url;

use crate::{WalkedFile, Walker};

/// Infrastructure trait for accessing environment configuration, system
/// variables, and persisted application configuration.
pub trait EnvironmentInfra: Send + Sync {
    /// The fully-resolved configuration type stored by the implementation.
    type Config: Clone + Send + Sync;

    fn get_env_var(&self, key: &str) -> Option<String>;
    fn get_env_vars(&self) -> BTreeMap<String, String>;

    /// Retrieves the current application configuration as an [`Environment`].
    fn get_environment(&self) -> Environment;

    /// Returns the latest fully-resolved configuration, re-reading from disk
    /// if a prior `update_environment` call has invalidated the cache.
    ///
    /// # Errors
    /// Returns an error if the disk read fails.
    fn get_config(&self) -> anyhow::Result<Self::Config>;

    /// Applies a list of configuration operations to the persisted config.
    ///
    /// Implementations should load the current config, apply each operation in
    /// order, and persist the result atomically.
    ///
    /// # Errors
    /// Returns an error if the configuration cannot be read or written.
    fn update_environment(
        &self,
        ops: Vec<ConfigOperation>,
    ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;
}

/// Repository for accessing system environment information
/// This uses the EnvironmentService trait from forge_domain
/// A service for reading files from the filesystem.
///
/// This trait provides an abstraction over file reading operations, allowing
/// for both real file system access and test mocking.
#[async_trait::async_trait]
pub trait FileReaderInfra: Send + Sync {
    /// Reads the content of a file at the specified path.
    /// Returns the file content as a UTF-8 string.
    async fn read_utf8(&self, path: &Path) -> anyhow::Result<String>;

    /// Reads multiple files in batches and returns a stream of file results.
    ///
    /// # Arguments
    /// * `batch_size` - Number of files to read concurrently per batch
    /// * `paths` - Vector of file paths to read
    ///
    /// Returns a stream where each item is a tuple containing (file_path,
    /// file_content). Files are processed in batches internally for concurrency
    /// control.
    fn read_batch_utf8(
        &self,
        batch_size: usize,
        paths: Vec<PathBuf>,
    ) -> impl futures::Stream<Item = (PathBuf, anyhow::Result<String>)> + Send;

    /// Reads the content of a file at the specified path.
    /// Returns the file content as raw bytes.
    async fn read(&self, path: &Path) -> anyhow::Result<Vec<u8>>;

    /// Reads a specific line range from a file at the specified path.
    /// Returns the file content within the range as a UTF-8 string along with
    /// metadata.
    ///
    /// - start_line specifies the starting line position (1-based, inclusive).
    /// - end_line specifies the ending line position (1-based, inclusive).
    /// - Both start_line and end_line are inclusive bounds.
    /// - Binary files are automatically detected and rejected.
    ///
    /// Returns a tuple containing the file content and FileInfo with metadata
    /// about the read operation:
    /// - FileInfo.start_line: starting line position
    /// - FileInfo.end_line: ending line position
    /// - FileInfo.total_lines: total line count in file
    /// - FileInfo.content_hash: SHA-256 hash of the **full** file content,
    ///   allowing callers to store a stable hash that matches what a whole-file
    ///   read produces (used by the external-change detector)
    async fn range_read_utf8(
        &self,
        path: &Path,
        start_line: u64,
        end_line: u64,
    ) -> anyhow::Result<(String, FileInfo)>;
}

#[async_trait::async_trait]
pub trait FileWriterInfra: Send + Sync {
    /// Writes the content of a file at the specified path.
    async fn write(&self, path: &Path, contents: Bytes) -> anyhow::Result<()>;

    /// Appends content to a file at the specified path, creating it if it does
    /// not exist.
    async fn append(&self, path: &Path, contents: Bytes) -> anyhow::Result<()>;

    /// Writes content to a temporary file with the given prefix and extension,
    /// and returns its path. The file will be kept (not deleted) after
    /// creation.
    ///
    /// # Arguments
    /// * `prefix` - Prefix for the temporary file name
    /// * `ext` - File extension (e.g. ".txt", ".md")
    /// * `content` - Content to write to the file
    async fn write_temp(&self, prefix: &str, ext: &str, content: &str) -> anyhow::Result<PathBuf>;
}

#[async_trait::async_trait]
pub trait FileRemoverInfra: Send + Sync {
    /// Removes a file at the specified path.
    async fn remove(&self, path: &Path) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
pub trait FileInfoInfra: Send + Sync {
    async fn is_binary(&self, path: &Path) -> Result<bool>;
    async fn is_file(&self, path: &Path) -> anyhow::Result<bool>;
    async fn exists(&self, path: &Path) -> anyhow::Result<bool>;
    async fn file_size(&self, path: &Path) -> anyhow::Result<u64>;
}

#[async_trait::async_trait]
pub trait FileDirectoryInfra {
    async fn create_dirs(&self, path: &Path) -> anyhow::Result<()>;
}

/// Service for executing shell commands
#[async_trait::async_trait]
pub trait CommandInfra: Send + Sync {
    /// Executes a shell command and returns the output
    async fn execute_command(
        &self,
        command: String,
        working_dir: PathBuf,
        silent: bool,
        env_vars: Option<Vec<String>>,
    ) -> anyhow::Result<CommandOutput>;

    /// execute the shell command on present stdio.
    async fn execute_command_raw(
        &self,
        command: &str,
        working_dir: PathBuf,
        env_vars: Option<Vec<String>>,
    ) -> anyhow::Result<std::process::ExitStatus>;
}

#[async_trait::async_trait]
pub trait UserInfra: Send + Sync {
    /// Prompts the user with question
    /// Returns None if the user interrupts the prompt
    async fn prompt_question(&self, question: &str) -> anyhow::Result<Option<String>>;

    /// Prompts the user to select a single option from a list
    /// Returns None if the user interrupts the selection
    async fn select_one<T: Clone + std::fmt::Display + Send + 'static>(
        &self,
        message: &str,
        options: Vec<T>,
    ) -> anyhow::Result<Option<T>>;

    /// Prompts the user to select a single option from an enum that implements
    /// IntoEnumIterator Returns None if the user interrupts the selection
    async fn select_one_enum<T>(&self, message: &str) -> anyhow::Result<Option<T>>
    where
        T: Clone + std::fmt::Display + Send + 'static + strum::IntoEnumIterator + std::str::FromStr,
        <T as std::str::FromStr>::Err: std::fmt::Debug,
    {
        let options: Vec<T> = T::iter().collect();
        let selected = self.select_one(message, options).await?;
        Ok(selected)
    }

    /// Prompts the user to select multiple options from a list
    /// Returns None if the user interrupts the selection
    async fn select_many<T: std::fmt::Display + Clone + Send + 'static>(
        &self,
        message: &str,
        options: Vec<T>,
    ) -> anyhow::Result<Option<Vec<T>>>;
}

#[async_trait::async_trait]
pub trait McpClientInfra: Clone + Send + Sync + 'static {
    async fn list(&self) -> anyhow::Result<Vec<ToolDefinition>>;
    async fn call(
        &self,
        tool_name: &ToolName,
        input: serde_json::Value,
    ) -> anyhow::Result<ToolOutput>;
}

#[async_trait::async_trait]
pub trait McpServerInfra: Send + Sync + 'static {
    type Client: McpClientInfra;
    async fn connect(
        &self,
        config: McpServerConfig,
        env_vars: &BTreeMap<String, String>,
        environment: &Environment,
    ) -> anyhow::Result<Self::Client>;
}
/// Service for walking filesystem directories
#[async_trait::async_trait]
pub trait WalkerInfra: Send + Sync {
    /// Walks the filesystem starting from the given directory with the
    /// specified configuration
    async fn walk(&self, config: Walker) -> anyhow::Result<Vec<WalkedFile>>;
}

/// HTTP service trait for making HTTP requests
#[async_trait::async_trait]
pub trait HttpInfra: Send + Sync + 'static {
    async fn http_get(&self, url: &Url, headers: Option<HeaderMap>) -> anyhow::Result<Response>;
    async fn http_post(
        &self,
        url: &Url,
        headers: Option<HeaderMap>,
        body: bytes::Bytes,
    ) -> anyhow::Result<Response>;
    async fn http_delete(&self, url: &Url) -> anyhow::Result<Response>;

    /// Posts JSON data and returns a server-sent events stream
    async fn http_eventsource(
        &self,
        url: &Url,
        headers: Option<HeaderMap>,
        body: Bytes,
    ) -> anyhow::Result<EventSource>;
}
/// Service for reading multiple files from a directory asynchronously
#[async_trait::async_trait]
pub trait DirectoryReaderInfra: Send + Sync {
    /// Lists all entries (files and directories) in a directory without reading
    /// file contents Returns a vector of tuples containing (entry_path,
    /// is_directory) This is much more efficient than read_directory_files
    /// when you only need to list entries
    async fn list_directory_entries(
        &self,
        directory: &Path,
    ) -> anyhow::Result<Vec<(PathBuf, bool)>>;

    /// Reads all files in a directory that match the given filter pattern
    /// Returns a vector of tuples containing (file_path, file_content)
    /// Files are read asynchronously/in parallel for better performance
    async fn read_directory_files(
        &self,
        directory: &Path,
        pattern: Option<&str>, // Optional glob pattern like "*.md"
    ) -> anyhow::Result<Vec<(PathBuf, String)>>;
}

/// Generic cache repository for content-addressable storage.
///
/// This trait provides an abstraction over caching operations with support for
/// arbitrary key and value types. Keys must be hashable and serializable, while
/// values must be serializable. The trait is designed to work with
/// content-addressable storage systems like cacache.
///
/// All operations return `anyhow::Result` for consistent error handling across
/// the infrastructure layer.
#[async_trait::async_trait]
pub trait KVStore: Send + Sync {
    /// Retrieves a value from the cache by its key.
    ///
    /// # Arguments
    /// * `key` - The key to look up in the cache
    ///
    /// # Errors
    /// Returns an error if the cache operation fails
    async fn cache_get<K, V>(&self, key: &K) -> Result<Option<V>>
    where
        K: Hash + Sync,
        V: serde::Serialize + DeserializeOwned + Send;

    /// Stores a value in the cache with the given key.
    ///
    /// If the key already exists, the value is overwritten.
    /// Uses content-addressable storage for integrity verification.
    ///
    /// # Arguments
    /// * `key` - The key to store the value under
    /// * `value` - The value to cache
    ///
    /// # Errors
    /// Returns an error if the cache operation fails
    async fn cache_set<K, V>(&self, key: &K, value: &V) -> Result<()>
    where
        K: Hash + Sync,
        V: serde::Serialize + Sync;

    /// Clears all entries from the cache.
    ///
    /// This operation removes all cached data. Use with caution.
    ///
    /// # Errors
    /// Returns an error if the cache clear operation fails
    async fn cache_clear(&self) -> Result<()>;
}

/// Provides HTTP features for OAuth authentication flows.
#[async_trait::async_trait]
pub trait OAuthHttpProvider: Send + Sync {
    /// Builds an authorization URL with provider-specific parameters.
    async fn build_auth_url(&self, config: &OAuthConfig) -> anyhow::Result<AuthCodeParams>;

    /// Exchanges an authorization code for an access token with
    /// provider-specific handling.
    async fn exchange_code(
        &self,
        config: &OAuthConfig,
        code: &str,
        verifier: Option<&str>,
    ) -> anyhow::Result<OAuthTokenResponse>;

    /// Creates an HTTP client with provider-specific headers and behavior.
    fn build_http_client(&self, config: &OAuthConfig) -> anyhow::Result<reqwest::Client>;
}

/// Authentication strategy trait
///
/// Defines the contract for authentication flows. Each strategy implements
/// the complete authentication lifecycle: initialization, completion, and
/// refresh.
#[async_trait::async_trait]
pub trait AuthStrategy: Send + Sync {
    /// Initialize authentication flow
    async fn init(&self) -> anyhow::Result<forge_domain::AuthContextRequest>;

    /// Complete authentication flow
    async fn complete(
        &self,
        context_response: forge_domain::AuthContextResponse,
    ) -> anyhow::Result<forge_domain::AuthCredential>;

    /// Refresh credential
    async fn refresh(
        &self,
        credential: &forge_domain::AuthCredential,
    ) -> anyhow::Result<forge_domain::AuthCredential>;
}

/// Factory trait for creating authentication strategies
///
/// Provides a way to create authentication strategies based on provider and
/// method configuration.
pub trait StrategyFactory: Send + Sync {
    type Strategy: AuthStrategy;
    fn create_auth_strategy(
        &self,
        provider_id: forge_domain::ProviderId,
        auth_method: forge_domain::AuthMethod,
        required_params: Vec<forge_domain::URLParamSpec>,
    ) -> anyhow::Result<Self::Strategy>;
}

/// Repository for loading agents from multiple sources.
///
/// This trait provides access to fully-resolved domain [`forge_domain::Agent`]
/// values from:
/// 1. Built-in agents (embedded in the application)
/// 2. Global custom agents (from ~/.forge/agents/ directory)
/// 3. Project-local agents (from .forge/agents/ directory in current working
///    directory)
///
/// ## Agent Precedence
/// When agents have duplicate IDs across different sources, the precedence
/// order is: **CWD (project-local) > Global custom > Built-in**
///
/// This means project-local agents can override global agents, and both can
/// override built-in agents.
#[async_trait::async_trait]
pub trait AgentRepository: Send + Sync {
    /// Load all agents from all available sources with conflict resolution.
    ///
    /// # Arguments
    ///
    /// * `provider_id` - Default provider applied to agents that do not specify
    ///   one
    /// * `model_id` - Default model applied to agents that do not specify one
    async fn get_agents(&self) -> anyhow::Result<Vec<forge_domain::Agent>>;

    /// Load lightweight metadata for all agents without requiring a configured
    /// provider or model.
    async fn get_agent_infos(&self) -> anyhow::Result<Vec<forge_domain::AgentInfo>>;
}

/// Infrastructure trait for providing shared gRPC channel
///
/// This trait provides access to a shared gRPC channel for communicating with
/// the workspace server. The channel is lazily connected and can be cloned
/// cheaply across multiple clients.
pub trait GrpcInfra: Send + Sync {
    /// Returns a cloned gRPC channel for the workspace server
    fn channel(&self) -> anyhow::Result<tonic::transport::Channel>;

    /// Hydrates the gRPC channel by establishing and then dropping the
    /// connection
    fn hydrate(&self);
}
