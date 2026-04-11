use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use forge_domain::{Environment, Snapshot, SnapshotRepository, UserInputId};

pub struct ForgeFileSnapshotService {
    inner: Arc<forge_snaps::SnapshotService>,
    snapshot_root: PathBuf,
}

impl ForgeFileSnapshotService {
    pub fn new(env: Environment) -> Self {
        let snapshot_root = env.snapshot_path();
        Self {
            inner: Arc::new(forge_snaps::SnapshotService::new(snapshot_root.clone())),
            snapshot_root,
        }
    }

    /// Returns the base directory under which all `.snap` files are stored.
    pub fn snapshot_root(&self) -> PathBuf {
        self.snapshot_root.clone()
    }
}

#[async_trait::async_trait]
impl SnapshotRepository for ForgeFileSnapshotService {
    // Creation
    async fn insert_snapshot(
        &self,
        file_path: &Path,
        user_input_id: UserInputId,
    ) -> Result<Snapshot> {
        self.inner
            .create_snapshot(file_path.to_path_buf(), user_input_id)
            .await
    }

    // Undo
    async fn undo_snapshot(&self, file_path: &Path) -> Result<()> {
        self.inner.undo_snapshot(file_path.to_path_buf()).await
    }
}
