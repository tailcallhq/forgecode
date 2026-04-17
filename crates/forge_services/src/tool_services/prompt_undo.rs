use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;
use forge_app::{FileReaderInfra, FileWriterInfra, PromptUndoOutput, PromptUndoService};
use forge_domain::{ConversationId, SnapshotMetadataRepository, UserInputId};

/// Restores all files that were changed during the most recent user prompt in a
/// conversation. Uses snapshot metadata from SQLite to identify which files
/// changed and where their pre-modification snapshots are stored on disk.
pub struct ForgePromptUndo<F> {
    infra: Arc<F>,
}

impl<F> ForgePromptUndo<F> {
    /// Creates a new `ForgePromptUndo` backed by the given infrastructure.
    pub fn new(infra: Arc<F>) -> Self {
        Self { infra }
    }
}

#[async_trait::async_trait]
impl<F: FileReaderInfra + FileWriterInfra + SnapshotMetadataRepository> PromptUndoService
    for ForgePromptUndo<F>
{
    async fn undo_last_prompt(
        &self,
        conversation_id: ConversationId,
    ) -> anyhow::Result<PromptUndoOutput> {
        // Step 1: Find all snapshots for this conversation, ordered by created_at
        // DESC.
        let conversation_snapshots = self
            .infra
            .find_snapshots_by_conversation_id(conversation_id)
            .await?;

        // Step 2: Determine the latest user_input_id from the first row.
        let latest_user_input_id = conversation_snapshots
            .first()
            .map(|(user_input_id, _, _)| user_input_id.clone())
            .ok_or_else(|| anyhow::anyhow!("No snapshots found for this conversation"))?;

        // Step 3: Fetch all (file_path, snap_file_path) pairs for that prompt.
        let user_input_id = UserInputId::parse(&latest_user_input_id)?;
        let file_snapshots = self
            .infra
            .find_snapshots_by_user_input_id(user_input_id)
            .await?;

        // Step 4: Restore each file from its snapshot.
        let mut restored_files = Vec::with_capacity(file_snapshots.len());
        for (file_path, snap_file_path) in &file_snapshots {
            let snap_path = Path::new(snap_file_path);
            let original_path = Path::new(file_path);

            let content = self.infra.read_utf8(snap_path).await?;
            self.infra
                .write(original_path, Bytes::from(content))
                .await?;
            restored_files.push(file_path.clone());
        }

        // Step 5: Mark those snapshot rows as undone.
        self.infra
            .mark_snapshots_undone(user_input_id)
            .await?;

        Ok(PromptUndoOutput { restored_files })
    }
}
