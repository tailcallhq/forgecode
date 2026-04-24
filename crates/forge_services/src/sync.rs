use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use forge_app::{FileReaderInfra, SyncProgressCounter, WorkspaceStatus, compute_hash};
use forge_domain::{ApiKey, FileHash, SyncProgress, UserId, WorkspaceId, WorkspaceIndexRepository};
use futures::stream::{Stream, StreamExt};
use tracing::{info, warn};

use crate::fd::{FileDiscovery, discover_sync_file_paths};

/// Error type for a single file that could not be read during workspace
/// operations, carrying the file path for downstream reporting.
#[derive(Debug, thiserror::Error)]
#[error("Failed to read file '{path}': {source}")]
struct FileReadError {
    path: PathBuf,
    #[source]
    source: anyhow::Error,
}

/// Canonicalizes `path`, attaching a context message that includes the original
/// path on failure.
pub fn canonicalize_path(path: PathBuf) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("Failed to resolve path: {}", path.display()))
}

/// Extracts [`forge_domain::FileStatus`] entries with
/// [`forge_domain::SyncStatus::Failed`] from a slice of file-read results by
/// downcasting errors to [`FileReadError`].
fn extract_failed_statuses<T>(results: &[Result<T>]) -> Vec<forge_domain::FileStatus> {
    results
        .iter()
        .filter_map(|r| r.as_ref().err())
        .filter_map(|e| e.downcast_ref::<FileReadError>())
        .map(|e| {
            forge_domain::FileStatus::new(
                e.path.to_string_lossy().into_owned(),
                forge_domain::SyncStatus::Failed,
            )
        })
        .collect()
}

/// Handles all sync operations for a workspace.
///
/// `F` provides infrastructure capabilities (file I/O, workspace index) and
/// `D` is the file-discovery strategy used to enumerate workspace files.
pub struct WorkspaceSyncEngine<F, D> {
    infra: Arc<F>,
    discovery: Arc<D>,
    workspace_root: PathBuf,
    workspace_id: WorkspaceId,
    user_id: UserId,
    token: ApiKey,
    batch_size: usize,
}

impl<F, D> WorkspaceSyncEngine<F, D> {
    /// Creates a new workspace sync engine with the provided infrastructure,
    /// file-discovery strategy, and workspace context shared by all operations.
    pub fn new(
        infra: Arc<F>,
        discovery: Arc<D>,
        workspace_root: PathBuf,
        workspace_id: WorkspaceId,
        user_id: UserId,
        token: ApiKey,
        batch_size: usize,
    ) -> Self {
        Self {
            infra,
            discovery,
            workspace_root,
            workspace_id,
            user_id,
            token,
            batch_size,
        }
    }
}

impl<F: 'static + WorkspaceIndexRepository + FileReaderInfra, D: FileDiscovery + 'static>
    WorkspaceSyncEngine<F, D>
{
    /// Executes the full workspace sync, emitting progress events via `emit`.
    ///
    /// Reads local file hashes, compares them against remote, then deletes
    /// stale files and uploads new or modified ones.
    pub async fn run<E, Fut>(&self, emit: E) -> Result<()>
    where
        E: Fn(SyncProgress) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = ()> + Send,
    {
        emit(SyncProgress::DiscoveringFiles {
            path: self.workspace_root.clone(),
            workspace_id: self.workspace_id.clone(),
        })
        .await;

        // Pass 1: stream files and collect only hashes — content is discarded
        // immediately after hashing so peak memory is bounded to one batch
        // of file content rather than the entire workspace.
        let results: Vec<Result<FileHash>> = self.read_hashes().collect().await;
        let failed_statuses = extract_failed_statuses(&results);
        let local_hashes: Vec<FileHash> = results.into_iter().flatten().collect();

        let total_file_count = local_hashes.len() + failed_statuses.len();
        emit(SyncProgress::FilesDiscovered { count: total_file_count }).await;

        let remote_files = self.fetch_remote_hashes().await?;

        emit(SyncProgress::ComparingFiles {
            remote_files: remote_files.len(),
            local_files: total_file_count,
        })
        .await;

        let plan = WorkspaceStatus::new(self.workspace_root.clone(), remote_files);
        let mut statuses = plan.file_statuses(local_hashes.clone());
        statuses.extend(failed_statuses);

        // Compute counts from statuses
        let added = statuses
            .iter()
            .filter(|s| s.status == forge_domain::SyncStatus::New)
            .count();
        let deleted = statuses
            .iter()
            .filter(|s| s.status == forge_domain::SyncStatus::Deleted)
            .count();
        let modified = statuses
            .iter()
            .filter(|s| s.status == forge_domain::SyncStatus::Modified)
            .count();
        let mut failed_files = statuses
            .iter()
            .filter(|s| s.status == forge_domain::SyncStatus::Failed)
            .count();

        // Compute total number of affected files
        let total_file_changes = added + deleted + modified;

        // Only emit diff computed event if there are actual changes
        if total_file_changes > 0 {
            emit(SyncProgress::DiffComputed { added, deleted, modified }).await;
        }

        // Derive the exact paths to delete/upload — no file content required
        let sync_paths = plan.get_sync_paths(local_hashes);

        let total_operations = sync_paths.delete.len() + sync_paths.upload.len();
        let mut counter = SyncProgressCounter::new(total_file_changes, total_operations);

        emit(counter.sync_progress()).await;

        // Delete all files in a single batched call
        match self.delete_files(sync_paths.delete.clone()).await {
            Ok(deleted_count) => {
                counter.complete(deleted_count);
                emit(counter.sync_progress()).await;
            }
            Err(e) => {
                warn!(workspace_id = %self.workspace_id, error = ?e, "Failed to delete files during sync");
                failed_files += sync_paths.delete.len();
            }
        }

        // Pass 2: upload files — files are grouped into batches of `batch_size`
        // and each batch is sent in a single HTTP request, sequentially.
        let mut upload_stream = Box::pin(self.upload_files(sync_paths.upload));

        // Process uploads as they complete, updating progress incrementally
        while let Some(result) = upload_stream.next().await {
            match result {
                Ok(count) => {
                    counter.complete(count);
                    emit(counter.sync_progress()).await;
                }
                Err(e) => {
                    warn!(workspace_id = %self.workspace_id, error = ?e, "Failed to upload file during sync");
                    failed_files += 1;
                    // Continue processing remaining uploads
                }
            }
        }

        info!(
            workspace_id = %self.workspace_id,
            total_files = total_file_count,
            "Sync completed successfully"
        );

        emit(SyncProgress::Completed {
            total_files: total_file_count,
            uploaded_files: total_file_changes,
            failed_files,
        })
        .await;

        // Fail if there were any failed files
        if failed_files > 0 {
            Err(forge_domain::Error::sync_failed(failed_files).into())
        } else {
            Ok(())
        }
    }

    /// Computes the current sync status for all files in the workspace.
    ///
    /// Reads local file hashes and compares them against the remote server to
    /// produce a per-file status report.
    pub async fn compute_status(&self) -> Result<Vec<forge_domain::FileStatus>> {
        let results: Vec<Result<FileHash>> = self.read_hashes().collect().await;

        let mut failed_statuses = extract_failed_statuses(&results);
        let local_hashes: Vec<FileHash> = results.into_iter().flatten().collect();

        let remote_files = self.fetch_remote_hashes().await?;

        let plan = WorkspaceStatus::new(self.workspace_root.clone(), remote_files);
        let mut statuses = plan.file_statuses(local_hashes);
        statuses.append(&mut failed_statuses);
        Ok(statuses)
    }

    /// Fetches remote file hashes from the server.
    async fn fetch_remote_hashes(&self) -> anyhow::Result<Vec<FileHash>> {
        info!(workspace_id = %self.workspace_id, "Fetching existing file hashes from server to detect changes...");
        let workspace_files =
            forge_domain::CodeBase::new(self.user_id.clone(), self.workspace_id.clone(), ());
        self.infra
            .list_workspace_files(&workspace_files, &self.token)
            .await
    }

    /// Deletes files from the workspace.
    ///
    /// Returns the number of files that were successfully deleted.
    async fn delete_files(&self, files_to_delete: Vec<PathBuf>) -> Result<usize> {
        if files_to_delete.is_empty() {
            return Ok(0);
        }

        let paths: Vec<String> = files_to_delete
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();

        let deletion =
            forge_domain::CodeBase::new(self.user_id.clone(), self.workspace_id.clone(), paths);
        self.infra
            .delete_files(&deletion, &self.token)
            .await
            .context("Failed to delete files")?;

        for path in &files_to_delete {
            info!(workspace_id = %self.workspace_id, path = %path.display(), "File deleted successfully");
        }

        Ok(files_to_delete.len())
    }

    /// Uploads files in batches, sending one HTTP request per batch of
    /// `batch_size` files.
    ///
    /// Files within each batch are read from disk, collected into a single
    /// [`forge_domain::FileUpload`] payload, and uploaded in one request.
    /// Batches are processed sequentially — only one HTTP request is in-flight
    /// at a time — which keeps both memory usage and server concurrency
    /// bounded. The stream yields the number of files successfully uploaded
    /// per batch.
    fn upload_files(
        &self,
        paths: Vec<PathBuf>,
    ) -> impl Stream<Item = Result<usize, anyhow::Error>> + Send {
        let user_id = self.user_id.clone();
        let workspace_id = self.workspace_id.clone();
        let token = self.token.clone();
        let infra = self.infra.clone();
        let batch_size = self.batch_size;

        futures::stream::iter(paths).chunks(batch_size).then(move |batch| {
            let user_id = user_id.clone();
            let workspace_id = workspace_id.clone();
            let token = token.clone();
            let infra = infra.clone();
            async move {
                let mut files = Vec::with_capacity(batch.len());
                for file_path in &batch {
                    info!(workspace_id = %workspace_id, path = %file_path.display(), "File sync started");
                    let content = infra
                        .read_utf8(file_path)
                        .await
                        .with_context(|| {
                            format!("Failed to read file '{}' for upload", file_path.display())
                        })?;
                    files.push(forge_domain::FileRead::new(
                        file_path.to_string_lossy().into_owned(),
                        content,
                    ));
                }
                let count = files.len();
                let upload = forge_domain::CodeBase::new(
                    user_id.clone(),
                    workspace_id.clone(),
                    files,
                );
                infra
                    .upload_files(&upload, &token)
                    .await
                    .context("Failed to upload files")?;
                for file_path in &batch {
                    info!(workspace_id = %workspace_id, path = %file_path.display(), "File sync completed");
                }
                Ok::<_, anyhow::Error>(count)
            }
        })
    }

    /// Discovers workspace files and streams their hashes without retaining
    /// file content in memory.
    ///
    /// Each file is read in batches for concurrency, but the content is
    /// discarded immediately after the hash is computed so that only one
    /// batch of file content occupies memory at a time.
    fn read_hashes(&self) -> impl Stream<Item = Result<FileHash>> + Send {
        let dir_path = self.workspace_root.clone();
        let infra = self.infra.clone();
        let discovery = self.discovery.clone();
        let workspace_id = self.workspace_id.clone();
        let batch_size = self.batch_size;

        async_stream::stream! {
            let file_paths: Vec<PathBuf> = match discover_sync_file_paths(
                discovery.as_ref(),
                &dir_path,
                &workspace_id,
            ).await {
                Ok(file_paths) => file_paths,
                Err(err) => {
                    yield Err(err);
                    return;
                }
            };

            let stream = infra.read_batch_utf8(batch_size, file_paths);
            futures::pin_mut!(stream);

            while let Some((absolute_path, result)) = stream.next().await {
                match result {
                    Ok(content) => {
                        let hash = compute_hash(&content);
                        // content is dropped here — only the hash is retained
                        let path_str = absolute_path.to_string_lossy().to_string();
                        yield Ok(FileHash { path: path_str, hash });
                    }
                    Err(e) => {
                        warn!(path = %absolute_path.display(), error = ?e, "Skipping unreadable file during sync");
                        yield Err(FileReadError { path: absolute_path, source: e }.into());
                    }
                }
            }
        }
    }
}
