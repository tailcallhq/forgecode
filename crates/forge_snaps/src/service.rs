use std::path::PathBuf;

use anyhow::{Context, Result};
use forge_domain::{ConversationId, Snapshot, UserInputId};
use forge_fs::ForgeFS;

/// Implementation of the SnapshotService
#[derive(Debug)]
pub struct SnapshotService {
    /// Base directory for storing snapshots
    snapshots_directory: PathBuf,
}

impl SnapshotService {
    /// Create a new FileSystemSnapshotService with a specific home path
    pub fn new(snapshot_base_dir: PathBuf) -> Self {
        Self { snapshots_directory: snapshot_base_dir }
    }
}

impl SnapshotService {
    pub async fn create_snapshot(
        &self,
        path: PathBuf,
        user_input_id: UserInputId,
        conversation_id: ConversationId,
    ) -> Result<Snapshot> {
        let snapshot = Snapshot::create(path, user_input_id, conversation_id)?;

        // Create intermediary directories if they don't exist
        let snapshot_path = snapshot.snapshot_path(Some(self.snapshots_directory.clone()));
        if let Some(parent) = PathBuf::from(&snapshot_path).parent() {
            ForgeFS::create_dir_all(parent).await?;
        }

        let content = ForgeFS::read(&snapshot.path).await?;
        let path = snapshot.snapshot_path(Some(self.snapshots_directory.clone()));
        ForgeFS::write(path, content).await?;
        Ok(snapshot)
    }

    /// Find the most recent snapshot for a given path based on filename
    /// timestamp
    async fn find_recent_snapshot(snapshot_dir: &PathBuf) -> Result<Option<PathBuf>> {
        let mut latest_path = None;
        let mut latest_filename = None;
        let mut dir = ForgeFS::read_dir(&snapshot_dir).await?;

        while let Some(entry) = dir.next_entry().await? {
            let filename = entry.file_name().to_string_lossy().to_string();
            if filename.ends_with(".snap")
                && (latest_filename.is_none() || filename > latest_filename.clone().unwrap())
            {
                latest_filename = Some(filename);
                latest_path = Some(entry.path());
            }
        }

        Ok(latest_path)
    }

    pub async fn undo_snapshot(&self, path: PathBuf) -> Result<()> {
        // Derive the per-file storage directory using a throwaway snapshot
        // (UserInputId here is irrelevant — we only need the path hash).
        let snapshot = Snapshot::create(path.clone(), UserInputId::new(), ConversationId::generate())?;

        // All the snaps for `path` are stored in `snapshot.path_hash()` directory.
        let snapshot_dir = self.snapshots_directory.join(snapshot.path_hash());

        // Check if the `snapshot_dir` exists
        if !ForgeFS::exists(&snapshot_dir) {
            return Err(anyhow::anyhow!("No snapshots found for {path:?}"));
        }

        // Retrieve the latest snapshot path
        let snapshot_path = Self::find_recent_snapshot(&snapshot_dir)
            .await?
            .context(format!("No valid snapshots found for {path:?}"))?;

        // Restore the content
        let content = ForgeFS::read(&snapshot_path).await?;
        ForgeFS::write(&path, content).await?;

        // Remove the used snapshot
        ForgeFS::remove_file(&snapshot_path).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    // Test helpers
    struct TestContext {
        _temp_dir: TempDir,
        _snapshots_dir: PathBuf,
        test_file: PathBuf,
        service: SnapshotService,
    }

    impl TestContext {
        async fn new() -> Result<Self> {
            let temp_dir = TempDir::new()?;
            let snapshots_dir = temp_dir.path().join("snapshots");
            // Canonicalize the temp directory path to ensure consistency
            let temp_path = temp_dir
                .path()
                .canonicalize()
                .unwrap_or_else(|_| temp_dir.path().to_path_buf());
            let test_file = temp_path.join("test.txt");
            let service = SnapshotService::new(snapshots_dir.clone());

            Ok(Self {
                _temp_dir: temp_dir,
                _snapshots_dir: snapshots_dir,
                test_file,
                service,
            })
        }

        async fn write_content(&self, content: &str) -> Result<()> {
            ForgeFS::write(&self.test_file, content.as_bytes()).await
        }

        async fn read_content(&self) -> Result<String> {
            let content = ForgeFS::read(&self.test_file).await?;
            Ok(String::from_utf8(content)?)
        }

        async fn create_snapshot(&self) -> Result<Snapshot> {
            self.service
                .create_snapshot(
                    self.test_file.clone(),
                    UserInputId::new(),
                    ConversationId::generate(),
                )
                .await
        }

        async fn undo_snapshot(&self) -> Result<()> {
            self.service.undo_snapshot(self.test_file.clone()).await
        }
    }

    #[tokio::test]
    async fn test_create_snapshot() -> Result<()> {
        // Arrange
        let ctx = TestContext::new().await?;
        let test_content = "Hello, World!";

        // Act
        ctx.write_content(test_content).await?;
        let snapshot = ctx.create_snapshot().await?;

        // Assert
        let snapshot_content = ForgeFS::read(&snapshot.path).await?;
        assert_eq!(String::from_utf8(snapshot_content)?, test_content);

        Ok(())
    }

    #[tokio::test]
    async fn test_undo_snapshot() -> Result<()> {
        // Arrange
        let ctx = TestContext::new().await?;
        let initial_content = "Initial content";
        let modified_content = "Modified content";

        // Act
        ctx.write_content(initial_content).await?;
        ctx.create_snapshot().await?;
        ctx.write_content(modified_content).await?;
        ctx.undo_snapshot().await?;

        // Assert
        assert_eq!(ctx.read_content().await?, initial_content);

        Ok(())
    }

    #[tokio::test]
    async fn test_undo_snapshot_no_snapshots() -> Result<()> {
        // Arrange
        let ctx = TestContext::new().await?;

        // Act
        ctx.write_content("test content").await?;
        let result = ctx.undo_snapshot().await;

        // Assert
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No snapshots found")
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_undo_snapshot_after_file_deletion() -> Result<()> {
        // Arrange
        let ctx = TestContext::new().await?;
        let initial_content = "Initial content";

        // Act
        ctx.write_content(initial_content).await?;
        ctx.create_snapshot().await?;
        ForgeFS::remove_file(&ctx.test_file).await?;

        // Assert - undo should succeed and recreate the file from snapshot
        ctx.undo_snapshot().await?;
        assert_eq!(ctx.read_content().await?, initial_content);

        Ok(())
    }

    #[tokio::test]
    async fn test_multiple_snapshots() -> Result<()> {
        // Arrange
        let ctx = TestContext::new().await?;

        // Act
        ctx.write_content("Initial content").await?;
        ctx.create_snapshot().await?;

        ctx.write_content("Second content").await?;
        ctx.create_snapshot().await?;

        ctx.write_content("Final content").await?;
        ctx.undo_snapshot().await?;

        // Assert
        assert_eq!(ctx.read_content().await?, "Second content");

        Ok(())
    }

    #[tokio::test]
    async fn test_multiple_snapshots_undo_twice() -> Result<()> {
        // Arrange
        let ctx = TestContext::new().await?;

        // Act
        ctx.write_content("Initial content").await?;
        ctx.create_snapshot().await?;

        ctx.write_content("Second content").await?;
        ctx.create_snapshot().await?;

        ctx.write_content("Final content").await?;
        ctx.undo_snapshot().await?;
        ctx.undo_snapshot().await?;

        // Assert
        assert_eq!(ctx.read_content().await?, "Initial content");

        Ok(())
    }

    #[tokio::test]
    async fn test_snapshot_filename_contains_user_input_id() -> Result<()> {
        // Arrange
        let ctx = TestContext::new().await?;
        let user_input_id = UserInputId::new();

        // Act
        ctx.write_content("some content").await?;
        ctx.service
            .create_snapshot(ctx.test_file.clone(), user_input_id, ConversationId::generate())
            .await?;

        // Assert: find the .snap file and verify the UUID is in its name
        let snapshot_dir = ctx
            ._snapshots_dir
            .read_dir()?
            .filter_map(|e| e.ok())
            .next()
            .expect("snapshot subdirectory should exist")
            .path();

        let snap_file = snapshot_dir
            .read_dir()?
            .filter_map(|e| e.ok())
            .next()
            .expect("snap file should exist");

        let filename = snap_file.file_name().to_string_lossy().to_string();
        assert!(
            filename.contains(&user_input_id.to_string()),
            "filename '{filename}' should contain user_input_id '{user_input_id}'"
        );
        assert!(filename.ends_with(".snap"));

        Ok(())
    }
}
