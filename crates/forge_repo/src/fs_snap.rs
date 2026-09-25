use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use forge_domain::{Environment, Snapshot, SnapshotRepository};

pub struct ForgeFileSnapshotService {
    cwd: std::path::PathBuf,
    base: std::path::PathBuf,
    inner: Arc<forge_snaps::SnapshotService>,
}

impl ForgeFileSnapshotService {
    pub fn new(env: Environment) -> Self {
        Self {
            cwd: env.cwd.clone(),
            base: env.snapshot_path(),
            inner: Arc::new(forge_snaps::SnapshotService::new(env.snapshot_path())),
        }
    }
}

#[async_trait::async_trait]
impl SnapshotRepository for ForgeFileSnapshotService {
    // Creation
    async fn insert_snapshot(&self, file_path: &Path) -> Result<Snapshot> {
        let base = self.base.clone();
        let cwd = self.cwd.clone();
        let path = file_path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            forge_snaps::ConversationHistory::declare(&base, &cwd, &path)
        })
        .await??;
        self.inner.create_snapshot(file_path.to_path_buf()).await
    }

    // Undo
    async fn undo_snapshot(&self, file_path: &Path) -> Result<()> {
        self.inner.undo_snapshot(file_path.to_path_buf()).await
    }
}
