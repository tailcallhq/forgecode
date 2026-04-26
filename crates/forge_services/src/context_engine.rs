use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use forge_app::{CommandInfra, EnvironmentInfra, FileReaderInfra, WalkerInfra, WorkspaceService};
use forge_domain::{
    AuthCredential, AuthDetails, ProviderId, ProviderRepository, SyncProgress, UserId, WorkspaceId,
    WorkspaceIndexRepository,
};
use forge_stream::MpscStream;
use futures::future::join_all;
use tracing::info;

use crate::fd::FileDiscovery;
use crate::sync::{WorkspaceSyncEngine, canonicalize_path};

/// Service for indexing workspaces and performing semantic search.
///
/// `F` provides infrastructure capabilities (file I/O, environment, etc.) and
/// `D` is the file-discovery strategy used to enumerate workspace files.
pub struct ForgeWorkspaceService<F, D> {
    infra: Arc<F>,
    discovery: Arc<D>,
}

impl<F, D> Clone for ForgeWorkspaceService<F, D> {
    fn clone(&self) -> Self {
        Self {
            infra: Arc::clone(&self.infra),
            discovery: Arc::clone(&self.discovery),
        }
    }
}

impl<F, D> ForgeWorkspaceService<F, D> {
    /// Creates a new workspace service with the provided infrastructure and
    /// file-discovery strategy.
    pub fn new(infra: Arc<F>, discovery: Arc<D>) -> Self {
        Self { infra, discovery }
    }
}

impl<
    F: 'static
        + ProviderRepository
        + WorkspaceIndexRepository
        + FileReaderInfra
        + EnvironmentInfra<Config = forge_config::ForgeConfig>
        + CommandInfra
        + WalkerInfra,
    D: FileDiscovery + 'static,
> ForgeWorkspaceService<F, D>
{
    /// Internal sync implementation that emits progress events.
    async fn sync_codebase_internal<E, Fut>(&self, path: PathBuf, emit: E) -> Result<()>
    where
        E: Fn(SyncProgress) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = ()> + Send,
    {
        info!(path = %path.display(), "Starting workspace sync");

        emit(SyncProgress::Starting).await;

        let (token, user_id) = self.get_workspace_credentials().await?;
        let batch_size = self.infra.get_config()?.max_file_read_batch_size;
        let path = canonicalize_path(path)?;

        // Find existing workspace - do NOT auto-create
        let workspace = self.get_workspace_by_path(path, &token).await?;
        let workspace_id = workspace.workspace_id.clone();

        // Use the canonical root stored in the workspace record so that file
        // discovery and remote-hash comparison are always relative to the same
        // base, even when `path` is a subdirectory of an ancestor workspace.
        let workspace_root = PathBuf::from(&workspace.working_dir);

        WorkspaceSyncEngine::new(
            Arc::clone(&self.infra),
            Arc::clone(&self.discovery),
            workspace_root,
            workspace_id,
            user_id,
            token,
            batch_size,
        )
        .run(emit)
        .await
    }

    /// Gets the ForgeCode services credential and extracts workspace auth
    /// components
    ///
    /// # Errors
    /// Returns an error if the credential is not found, if there's a database
    /// error, or if the credential format is invalid
    async fn get_workspace_credentials(&self) -> Result<(forge_domain::ApiKey, UserId)> {
        let credential = self
            .infra
            .get_credential(&ProviderId::FORGE_SERVICES)
            .await?
            .context("No authentication credentials found. Please authenticate first.")?;

        match &credential.auth_details {
            AuthDetails::ApiKey(token) => {
                // Extract user_id from URL params
                let user_id_str = credential
                    .url_params
                    .get(&"user_id".to_string().into())
                    .ok_or_else(|| {
                        anyhow::anyhow!("Missing user_id in ForgeServices credential")
                    })?;
                let user_id = UserId::from_string(user_id_str.as_str())?;

                Ok((token.clone(), user_id))
            }
            _ => anyhow::bail!("ForgeServices credential must be an API key"),
        }
    }

    /// Finds a workspace by path from remote server, checking for exact match
    /// first, then ancestor workspaces.
    ///
    /// Business logic:
    /// 1. First tries to find an exact match for the given path
    /// 2. If not found, searches for ancestor workspaces
    /// 3. Returns the closest ancestor (longest matching path prefix)
    ///
    /// # Errors
    /// Returns an error if the path cannot be canonicalized or if there's a
    /// server error. Returns Ok(None) if no workspace is found.
    async fn find_workspace_by_path(
        &self,
        path: PathBuf,
        token: &forge_domain::ApiKey,
    ) -> Result<Option<forge_domain::WorkspaceInfo>> {
        let canonical_path = canonicalize_path(path)?;

        // Get all workspaces from remote server
        let workspaces = self.infra.list_workspaces(token).await?;

        let canonical_str = canonical_path.to_string_lossy();

        // Business logic: choose which workspace to use
        // 1. First check for exact match
        if let Some(exact_match) = workspaces.iter().find(|w| w.working_dir == canonical_str) {
            return Ok(Some(exact_match.clone()));
        }

        // 2. Find closest ancestor (longest matching path prefix)
        let mut best_match: Option<(&forge_domain::WorkspaceInfo, usize)> = None;

        for workspace in &workspaces {
            let workspace_path = PathBuf::from(&workspace.working_dir);
            if canonical_path.starts_with(&workspace_path) {
                let path_len = workspace.working_dir.len();
                if best_match.is_none_or(|(_, len)| path_len > len) {
                    best_match = Some((workspace, path_len));
                }
            }
        }

        Ok(best_match.map(|(w, _)| w.clone()))
    }

    /// Looks up the workspace for `path` and returns it, or an error if no
    /// workspace has been indexed for that path.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying repository lookup fails, or when no
    /// matching workspace is found (i.e. the workspace has not been indexed
    /// yet).
    async fn get_workspace_by_path(
        &self,
        path: PathBuf,
        token: &forge_domain::ApiKey,
    ) -> Result<forge_domain::WorkspaceInfo> {
        self.find_workspace_by_path(path, token)
            .await?
            .context("Workspace not indexed. Please run `forge workspace init` first.")
    }

    async fn _init_workspace(&self, path: PathBuf) -> Result<(bool, WorkspaceId)> {
        let (token, _user_id) = self.get_workspace_credentials().await?;
        let path = canonicalize_path(path)?;

        // Find workspace by exact match or ancestor from remote server
        let workspace = self.find_workspace_by_path(path.clone(), &token).await?;

        let (workspace_id, workspace_path, is_new_workspace) = match workspace {
            Some(workspace_info) => {
                // Found existing workspace - reuse it
                (workspace_info.workspace_id, path.clone(), false)
            }
            None => {
                // No workspace found - create new
                (WorkspaceId::generate(), path.clone(), true)
            }
        };

        let workspace_id = if is_new_workspace {
            // Create workspace on server
            self.infra
                .create_workspace(&workspace_path, &token)
                .await
                .context("Failed to create workspace on server")?
        } else {
            workspace_id
        };

        Ok((is_new_workspace, workspace_id))
    }
}

#[async_trait]
impl<
    F: ProviderRepository
        + WorkspaceIndexRepository
        + FileReaderInfra
        + EnvironmentInfra<Config = forge_config::ForgeConfig>
        + CommandInfra
        + WalkerInfra
        + 'static,
    D: FileDiscovery + 'static,
> WorkspaceService for ForgeWorkspaceService<F, D>
{
    async fn sync_workspace(&self, path: PathBuf) -> Result<MpscStream<Result<SyncProgress>>> {
        let service = Clone::clone(self);

        let stream = MpscStream::spawn(move |tx| async move {
            // Create emit closure that captures the sender
            let emit = |progress: SyncProgress| {
                let tx = tx.clone();
                async move {
                    let _ = tx.send(Ok(progress)).await;
                }
            };

            // Run the sync and emit progress events
            let result = service.sync_codebase_internal(path, emit).await;

            // If there was an error, send it through the channel
            if let Err(e) = result {
                let _ = tx.send(Err(e)).await;
            }
        });

        Ok(stream)
    }

    /// Performs semantic code search on a workspace.
    async fn query_workspace(
        &self,
        path: PathBuf,
        params: forge_domain::SearchParams<'_>,
    ) -> Result<Vec<forge_domain::Node>> {
        let (token, user_id) = self.get_workspace_credentials().await?;

        let workspace = self
            .find_workspace_by_path(path, &token)
            .await?
            .ok_or(forge_domain::Error::WorkspaceNotFound)?;

        let search_query =
            forge_domain::CodeBase::new(user_id, workspace.workspace_id.clone(), params);

        let results = self
            .infra
            .search(&search_query, &token)
            .await
            .context("Failed to search")?;

        Ok(results)
    }

    /// Lists all workspaces.
    async fn list_workspaces(&self) -> Result<Vec<forge_domain::WorkspaceInfo>> {
        let (token, _) = self.get_workspace_credentials().await?;

        self.infra
            .as_ref()
            .list_workspaces(&token)
            .await
            .context("Failed to list workspaces")
    }

    /// Retrieves workspace information for a specific path.
    async fn get_workspace_info(
        &self,
        path: PathBuf,
    ) -> Result<Option<forge_domain::WorkspaceInfo>> {
        let (token, _user_id) = self.get_workspace_credentials().await?;
        let workspace = self.find_workspace_by_path(path, &token).await?;

        Ok(workspace)
    }

    /// Deletes a workspace from the server.
    async fn delete_workspace(&self, workspace_id: &forge_domain::WorkspaceId) -> Result<()> {
        let (token, _) = self.get_workspace_credentials().await?;

        self.infra
            .as_ref()
            .delete_workspace(workspace_id, &token)
            .await
            .context("Failed to delete workspace from server")?;

        Ok(())
    }

    /// Deletes multiple workspaces in parallel from both the server and local
    /// database.
    async fn delete_workspaces(&self, workspace_ids: &[forge_domain::WorkspaceId]) -> Result<()> {
        // Delete all workspaces in parallel by calling delete_workspace for each
        let delete_tasks: Vec<_> = workspace_ids
            .iter()
            .map(|workspace_id| self.delete_workspace(workspace_id))
            .collect();

        let results = join_all(delete_tasks).await;

        // Collect all errors
        let errors: Vec<_> = results.into_iter().filter_map(|r| r.err()).collect();

        if !errors.is_empty() {
            return Err(anyhow::anyhow!(
                "Failed to delete {} workspace(s): [{}]",
                errors.len(),
                errors
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        Ok(())
    }

    async fn is_indexed(&self, path: &std::path::Path) -> Result<bool> {
        let (token, _user_id) = self.get_workspace_credentials().await?;
        match self
            .find_workspace_by_path(path.to_path_buf(), &token)
            .await
        {
            Ok(workspace) => Ok(workspace.is_some()),
            Err(_) => Ok(false), // Path doesn't exist or other error, so it can't be indexed
        }
    }

    async fn get_workspace_status(&self, path: PathBuf) -> Result<Vec<forge_domain::FileStatus>> {
        let (token, user_id) = self.get_workspace_credentials().await?;

        let workspace = self.get_workspace_by_path(path, &token).await?;

        // Reuse the canonical path already stored in the workspace (resolved during
        // sync), avoiding a redundant canonicalize() IO call.
        let canonical_path = PathBuf::from(&workspace.working_dir);

        let batch_size = self.infra.get_config()?.max_file_read_batch_size;

        WorkspaceSyncEngine::new(
            Arc::clone(&self.infra),
            Arc::clone(&self.discovery),
            canonical_path,
            workspace.workspace_id,
            user_id,
            token,
            batch_size,
        )
        .compute_status()
        .await
    }

    async fn is_authenticated(&self) -> Result<bool> {
        Ok(self
            .infra
            .get_credential(&ProviderId::FORGE_SERVICES)
            .await?
            .is_some())
    }

    async fn init_auth_credentials(&self) -> Result<forge_domain::WorkspaceAuth> {
        // Authenticate with the indexing service
        let auth = self
            .infra
            .authenticate()
            .await
            .context("Failed to authenticate with indexing service")?;

        // Convert to AuthCredential and store
        let mut url_params = HashMap::new();
        url_params.insert(
            "user_id".to_string().into(),
            auth.user_id.to_string().into(),
        );

        let credential = AuthCredential {
            id: ProviderId::FORGE_SERVICES,
            auth_details: auth.clone().into(),
            url_params,
        };

        self.infra
            .upsert_credential(credential)
            .await
            .context("Failed to store authentication credentials")?;

        Ok(auth)
    }

    async fn init_workspace(&self, path: PathBuf) -> Result<WorkspaceId> {
        let (is_new, workspace_id) = self._init_workspace(path).await?;

        if is_new {
            Ok(workspace_id)
        } else {
            Err(forge_domain::Error::WorkspaceAlreadyInitialized(workspace_id).into())
        }
    }
}
