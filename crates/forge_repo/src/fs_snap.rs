use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use forge_domain::{Environment, Snapshot, SnapshotRepository, UserInputId};

pub struct ForgeFileSnapshotService {
    inner: Arc<forge_snaps::SnapshotService>,
}

impl ForgeFileSnapshotService {
    pub fn new(env: Environment) -> Self {
        Self {
            inner: Arc::new(forge_snaps::SnapshotService::new(env.snapshot_path())),
        }
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
