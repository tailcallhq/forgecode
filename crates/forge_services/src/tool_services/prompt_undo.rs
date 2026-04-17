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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use bytes::Bytes;
    use forge_app::{FileReaderInfra, FileWriterInfra, PromptUndoOutput, PromptUndoService};
    use forge_domain::{ConversationId, FileInfo, SnapshotMetadataRepository, UserInputId};
    use pretty_assertions::assert_eq;

    use super::*;

    /// Mock infrastructure for testing `ForgePromptUndo`.
    /// Stores files in-memory and tracks snapshot metadata.
    struct MockUndoInfra {
        files: Mutex<HashMap<String, String>>,
        conversation_snapshots: Mutex<Vec<(String, String, String)>>,
        user_input_snapshots: Mutex<HashMap<String, Vec<(String, String)>>>,
        undone: Mutex<Vec<String>>,
    }

    impl MockUndoInfra {
        fn new() -> Self {
            Self {
                files: Mutex::new(HashMap::new()),
                conversation_snapshots: Mutex::new(Vec::new()),
                user_input_snapshots: Mutex::new(HashMap::new()),
                undone: Mutex::new(Vec::new()),
            }
        }

        fn add_snapshot(
            &self,
            _conversation_id: ConversationId,
            user_input_id: &str,
            file_path: &str,
            snap_file_path: &str,
            snap_content: &str,
        ) {
            self.files
                .lock()
                .unwrap()
                .insert(snap_file_path.to_string(), snap_content.to_string());

            self.conversation_snapshots
                .lock()
                .unwrap()
                .push((
                    user_input_id.to_string(),
                    file_path.to_string(),
                    snap_file_path.to_string(),
                ));

            self.user_input_snapshots
                .lock()
                .unwrap()
                .entry(user_input_id.to_string())
                .or_default()
                .push((file_path.to_string(), snap_file_path.to_string()));
        }
    }

    #[async_trait::async_trait]
    impl FileReaderInfra for MockUndoInfra {
        async fn read_utf8(&self, path: &Path) -> anyhow::Result<String> {
            self.files
                .lock()
                .unwrap()
                .get(path.to_str().unwrap())
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("File not found: {}", path.display()))
        }

        fn read_batch_utf8(
            &self,
            _batch_size: usize,
            _paths: Vec<PathBuf>,
        ) -> impl futures::Stream<Item = (PathBuf, anyhow::Result<String>)> + Send {
            futures::stream::empty()
        }

        async fn read(&self, _path: &Path) -> anyhow::Result<Vec<u8>> {
            unimplemented!()
        }

        async fn range_read_utf8(
            &self,
            _path: &Path,
            _start_line: u64,
            _end_line: u64,
        ) -> anyhow::Result<(String, FileInfo)> {
            unimplemented!()
        }
    }

    #[async_trait::async_trait]
    impl FileWriterInfra for MockUndoInfra {
        async fn write(&self, path: &Path, contents: Bytes) -> anyhow::Result<()> {
            self.files.lock().unwrap().insert(
                path.to_str().unwrap().to_string(),
                String::from_utf8(contents.to_vec())?,
            );
            Ok(())
        }

        async fn append(&self, _path: &Path, _contents: Bytes) -> anyhow::Result<()> {
            unimplemented!()
        }

        async fn write_temp(
            &self,
            _prefix: &str,
            _ext: &str,
            _content: &str,
        ) -> anyhow::Result<PathBuf> {
            unimplemented!()
        }
    }

    #[async_trait::async_trait]
    impl SnapshotMetadataRepository for MockUndoInfra {
        async fn insert_snapshot_metadata(
            &self,
            _snapshot: &forge_domain::Snapshot,
            _snap_file_path: String,
        ) -> anyhow::Result<()> {
            unimplemented!()
        }

        async fn find_snapshots_by_user_input_id(
            &self,
            user_input_id: UserInputId,
        ) -> anyhow::Result<Vec<(String, String)>> {
            Ok(self
                .user_input_snapshots
                .lock()
                .unwrap()
                .get(&user_input_id.to_string())
                .cloned()
                .unwrap_or_default())
        }

        async fn find_snapshots_by_conversation_id(
            &self,
            _conversation_id: ConversationId,
        ) -> anyhow::Result<Vec<(String, String, String)>> {
            Ok(self.conversation_snapshots.lock().unwrap().clone())
        }

        async fn mark_snapshots_undone(&self, user_input_id: UserInputId) -> anyhow::Result<()> {
            self.undone
                .lock()
                .unwrap()
                .push(user_input_id.to_string());
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_undo_last_prompt_restores_single_file() {
        let conversation_id = ConversationId::generate();
        let user_input_id = UserInputId::new();

        let fixture = MockUndoInfra::new();
        fixture.add_snapshot(
            conversation_id,
            &user_input_id.to_string(),
            "/home/user/test.txt",
            "/tmp/snaps/test.txt.snap",
            "original content",
        );

        let service = ForgePromptUndo::new(Arc::new(fixture));
        let actual = service.undo_last_prompt(conversation_id).await.unwrap();

        let expected = PromptUndoOutput {
            restored_files: vec!["/home/user/test.txt".to_string()],
        };
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_undo_last_prompt_restores_multiple_files() {
        let conversation_id = ConversationId::generate();
        let user_input_id = UserInputId::new();

        let fixture = MockUndoInfra::new();
        fixture.add_snapshot(
            conversation_id,
            &user_input_id.to_string(),
            "/home/user/file1.txt",
            "/tmp/snaps/file1.txt.snap",
            "content1",
        );
        fixture.add_snapshot(
            conversation_id,
            &user_input_id.to_string(),
            "/home/user/file2.rs",
            "/tmp/snaps/file2.rs.snap",
            "content2",
        );

        let service = ForgePromptUndo::new(Arc::new(fixture));
        let actual = service.undo_last_prompt(conversation_id).await.unwrap();

        let expected = PromptUndoOutput {
            restored_files: vec![
                "/home/user/file1.txt".to_string(),
                "/home/user/file2.rs".to_string(),
            ],
        };
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_undo_last_prompt_no_snapshots_returns_error() {
        let conversation_id = ConversationId::generate();
        let fixture = MockUndoInfra::new();

        let service = ForgePromptUndo::new(Arc::new(fixture));
        let result = service.undo_last_prompt(conversation_id).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_undo_last_prompt_marks_snapshots_undone() {
        let conversation_id = ConversationId::generate();
        let user_input_id = UserInputId::new();

        let fixture = MockUndoInfra::new();
        fixture.add_snapshot(
            conversation_id,
            &user_input_id.to_string(),
            "/home/user/test.txt",
            "/tmp/snaps/test.txt.snap",
            "original content",
        );

        let service = ForgePromptUndo::new(Arc::new(fixture));
        service.undo_last_prompt(conversation_id).await.unwrap();

        let infra = service.infra.as_ref();
        let undone = infra.undone.lock().unwrap();
        assert_eq!(undone.len(), 1);
        assert_eq!(undone[0], user_input_id.to_string());
    }
}
