use std::path::Path;
use std::sync::Arc;

use forge_app::{FileReaderInfra, FileRemoverInfra, FsRemoveOutput, FsRemoveService};
use forge_domain::SnapshotRepository;

use crate::utils::assert_absolute_path;

/// Service for removing files with snapshot coordination
///
/// This service coordinates between infrastructure (file I/O) and repository
/// (snapshots) to remove files while preserving the ability to undo the
/// deletion.
pub struct ForgeFsRemove<F> {
    infra: Arc<F>,
}

impl<F> ForgeFsRemove<F> {
    pub fn new(infra: Arc<F>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<F: FileReaderInfra + FileRemoverInfra + SnapshotRepository> FsRemoveService
    for ForgeFsRemove<F>
{
    async fn remove(
        &self,
        input_path: String,
        user_input_id: forge_domain::UserInputId,
        conversation_id: forge_domain::ConversationId,
    ) -> anyhow::Result<FsRemoveOutput> {
        let path = Path::new(&input_path);
        assert_absolute_path(path)?;

        let content = self.infra.read_utf8(path).await.unwrap_or_default();

        // SNAPSHOT COORDINATION: Always capture snapshot before removing
        self.infra
            .insert_snapshot(path, user_input_id, conversation_id)
            .await?;

        self.infra.remove(path).await?;

        Ok(FsRemoveOutput { content })
    }
}
