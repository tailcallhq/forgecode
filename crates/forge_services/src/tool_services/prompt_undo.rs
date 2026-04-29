use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;
use forge_app::{
    FileReaderInfra, FileRemoverInfra, FileWriterInfra, PromptUndoOutput, PromptUndoService,
};
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
impl<F: FileReaderInfra + FileWriterInfra + FileRemoverInfra + SnapshotMetadataRepository>
    PromptUndoService for ForgePromptUndo<F>
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

        // Step 2: If there are no active snapshots, return an empty result
        // rather than an error. This makes the API ergonomic: calling /undo
        // when there's nothing to undo simply reports "no changes".
        let latest_user_input_id = match conversation_snapshots.first() {
            Some((user_input_id, _, _)) => user_input_id.clone(),
            None => {
                return Ok(PromptUndoOutput::default());
            }
        };

        // Step 3: Fetch all (file_path, snap_file_path) pairs for that prompt.
        let user_input_id = UserInputId::parse(&latest_user_input_id)?;
        let file_snapshots = self
            .infra
            .find_snapshots_by_user_input_id(user_input_id)
            .await?;

        // Step 4: Restore or delete each file based on its snapshot type.
        let mut restored_files = Vec::with_capacity(file_snapshots.len());
        let mut deleted_files = Vec::new();
        for (file_path, snap_file_path) in &file_snapshots {
            if snap_file_path.is_empty() {
                // New file created during prompt: delete it on undo.
                // Tolerate NotFound — if the file was already manually deleted,
                // the desired end state is already achieved.
                if let Err(err) = self.infra.remove(Path::new(file_path)).await
                    && !is_not_found(&err)
                {
                    return Err(err);
                }
                deleted_files.push(file_path.clone());
            } else {
                // Existing file modified: restore from snapshot.
                let snap_path = Path::new(snap_file_path);
                let original_path = Path::new(file_path);

                let content = self.infra.read_utf8(snap_path).await?;
                self.infra
                    .write(original_path, Bytes::from(content))
                    .await?;
                restored_files.push(file_path.clone());
            }
        }

        // Step 5: Mark those snapshot rows as undone.
        self.infra.mark_snapshots_undone(user_input_id).await?;

        Ok(PromptUndoOutput { restored_files, deleted_files })
    }
}

/// Checks whether an error is caused by a file-not-found condition.
/// This handles both `std::io::ErrorKind::NotFound` directly and when it's
/// wrapped inside an `anyhow::Error`.
fn is_not_found(err: &anyhow::Error) -> bool {
    err.downcast_ref::<std::io::Error>()
        .map(|e| e.kind() == std::io::ErrorKind::NotFound)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use bytes::Bytes;
    use forge_app::{
        FileReaderInfra, FileRemoverInfra, FileWriterInfra, PromptUndoOutput, PromptUndoService,
    };
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
        removed: Mutex<Vec<String>>,
    }

    impl MockUndoInfra {
        fn new() -> Self {
            Self {
                files: Mutex::new(HashMap::new()),
                conversation_snapshots: Mutex::new(Vec::new()),
                user_input_snapshots: Mutex::new(HashMap::new()),
                undone: Mutex::new(Vec::new()),
                removed: Mutex::new(Vec::new()),
            }
        }

        fn add_snapshot(
            &self,
            _conversation_id: ConversationId,
            user_input_id: &str,
            file_path: &str,
            snap_file_path: &str,
            snap_content: &str,
            file_exists: bool,
        ) {
            if !snap_file_path.is_empty() {
                self.files
                    .lock()
                    .unwrap()
                    .insert(snap_file_path.to_string(), snap_content.to_string());
            }

            // For new files, optionally add the file to the mock filesystem
            // so the remove() call can find it.
            if file_exists {
                self.files
                    .lock()
                    .unwrap()
                    .insert(file_path.to_string(), "current content".to_string());
            }

            self.conversation_snapshots.lock().unwrap().push((
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
    impl FileRemoverInfra for MockUndoInfra {
        async fn remove(&self, path: &Path) -> anyhow::Result<()> {
            let path_str = path.to_str().unwrap().to_string();
            if !self.files.lock().unwrap().contains_key(&path_str) {
                return Err(anyhow::anyhow!(std::io::Error::from(
                    std::io::ErrorKind::NotFound
                )));
            }
            self.files.lock().unwrap().remove(&path_str);
            self.removed.lock().unwrap().push(path_str);
            Ok(())
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
            self.undone.lock().unwrap().push(user_input_id.to_string());
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
            false,
        );

        let service = ForgePromptUndo::new(Arc::new(fixture));
        let actual = service.undo_last_prompt(conversation_id).await.unwrap();

        let expected = PromptUndoOutput {
            restored_files: vec!["/home/user/test.txt".to_string()],
            deleted_files: vec![],
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
            false,
        );
        fixture.add_snapshot(
            conversation_id,
            &user_input_id.to_string(),
            "/home/user/file2.rs",
            "/tmp/snaps/file2.rs.snap",
            "content2",
            false,
        );

        let service = ForgePromptUndo::new(Arc::new(fixture));
        let actual = service.undo_last_prompt(conversation_id).await.unwrap();

        let expected = PromptUndoOutput {
            restored_files: vec![
                "/home/user/file1.txt".to_string(),
                "/home/user/file2.rs".to_string(),
            ],
            deleted_files: vec![],
        };
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_undo_last_prompt_no_snapshots_returns_empty() {
        let conversation_id = ConversationId::generate();
        let fixture = MockUndoInfra::new();

        let service = ForgePromptUndo::new(Arc::new(fixture));
        let actual = service.undo_last_prompt(conversation_id).await.unwrap();

        let expected = PromptUndoOutput::default();
        assert_eq!(actual, expected);
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
            false,
        );

        let service = ForgePromptUndo::new(Arc::new(fixture));
        service.undo_last_prompt(conversation_id).await.unwrap();

        let infra = service.infra.as_ref();
        let undone = infra.undone.lock().unwrap();
        assert_eq!(undone.len(), 1);
        assert_eq!(undone[0], user_input_id.to_string());
    }

    #[tokio::test]
    async fn test_undo_last_prompt_deletes_new_file() {
        let conversation_id = ConversationId::generate();
        let user_input_id = UserInputId::new();

        let fixture = MockUndoInfra::new();
        // New file has empty snap_file_path (no prior content to back up).
        // file_exists=true simulates the file still being on disk.
        fixture.add_snapshot(
            conversation_id,
            &user_input_id.to_string(),
            "/home/user/new_file.txt",
            "",
            "",
            true,
        );

        let service = ForgePromptUndo::new(Arc::new(fixture));
        let actual = service.undo_last_prompt(conversation_id).await.unwrap();

        let expected = PromptUndoOutput {
            restored_files: vec![],
            deleted_files: vec!["/home/user/new_file.txt".to_string()],
        };
        assert_eq!(actual, expected);

        // Verify the file was removed
        let infra = service.infra.as_ref();
        let removed = infra.removed.lock().unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], "/home/user/new_file.txt");
    }

    #[tokio::test]
    async fn test_undo_last_prompt_deletes_already_manually_deleted_new_file() {
        let conversation_id = ConversationId::generate();
        let user_input_id = UserInputId::new();

        let fixture = MockUndoInfra::new();
        // New file was already manually deleted (file_exists=false).
        // The undo should succeed silently — the desired end state is already
        // achieved.
        fixture.add_snapshot(
            conversation_id,
            &user_input_id.to_string(),
            "/home/user/new_file.txt",
            "",
            "",
            false, // file already manually deleted
        );

        let service = ForgePromptUndo::new(Arc::new(fixture));
        let actual = service.undo_last_prompt(conversation_id).await.unwrap();

        let expected = PromptUndoOutput {
            restored_files: vec![],
            deleted_files: vec!["/home/user/new_file.txt".to_string()],
        };
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_undo_last_prompt_restores_and_deletes_mixed() {
        let conversation_id = ConversationId::generate();
        let user_input_id = UserInputId::new();

        let fixture = MockUndoInfra::new();
        // Existing file modified: has a snapshot
        fixture.add_snapshot(
            conversation_id,
            &user_input_id.to_string(),
            "/home/user/existing.txt",
            "/tmp/snaps/existing.txt.snap",
            "original content",
            false,
        );
        // New file created: empty snap_file_path
        fixture.add_snapshot(
            conversation_id,
            &user_input_id.to_string(),
            "/home/user/brand_new.rs",
            "",
            "",
            true,
        );

        let service = ForgePromptUndo::new(Arc::new(fixture));
        let actual = service.undo_last_prompt(conversation_id).await.unwrap();

        let expected = PromptUndoOutput {
            restored_files: vec!["/home/user/existing.txt".to_string()],
            deleted_files: vec!["/home/user/brand_new.rs".to_string()],
        };
        assert_eq!(actual, expected);
    }
}
