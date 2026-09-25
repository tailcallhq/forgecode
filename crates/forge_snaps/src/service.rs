use std::path::PathBuf;

use anyhow::{Context, Result};
use forge_domain::Snapshot;
use forge_fs::ForgeFS;

/// Per-file snapshots backed by filesnap with legacy raw snapshot support.
#[derive(Debug)]
pub struct SnapshotService {
    /// Base directory for storing snapshots
    snapshots_directory: PathBuf,
    operations: tokio::sync::Mutex<()>,
}

impl SnapshotService {
    /// Create a per-file snapshot service using the host storage directory.
    ///
    /// # Arguments
    /// * `snapshot_base_dir` - Directory containing snapshot references and the
    ///   filesnap content store.
    pub fn new(snapshot_base_dir: PathBuf) -> Self {
        Self {
            snapshots_directory: snapshot_base_dir,
            operations: tokio::sync::Mutex::new(()),
        }
    }
}

impl SnapshotService {
    /// Capture a file in filesnap's content-addressed store.
    ///
    /// # Arguments
    /// * `path` - Local file whose contents or absence should be captured.
    ///
    /// # Errors
    /// Returns an error if the file cannot be captured or its reference cannot
    /// be persisted.
    pub async fn create_snapshot(&self, path: PathBuf) -> Result<Snapshot> {
        let _guard = self.operations.lock().await;
        let snapshot = Snapshot::create(path)?;
        let base = self.snapshots_directory.clone();
        let captured = snapshot.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            std::fs::create_dir_all(&base)?;
            let source = PathBuf::from(&captured.path);
            anyhow::ensure!(
                !source.starts_with(base.canonicalize()?),
                "Cannot snapshot the snapshot store"
            );
            // Forge's contract is one explicitly requested file. This stable host
            // partition deliberately does not scan the containing directory.
            Self::cleanup_retired(&base, &captured.path_hash());
            let store = filesnap::WorkspaceStore::open(&base, &base)?;
            let checkpoint =
                store.checkpoint(&captured.id.to_string(), &captured.id.to_string(), [source])?;
            anyhow::ensure!(
                checkpoint.stats.dropped == 0,
                "Could not capture the requested file"
            );
            let record = captured
                .snapshot_path(Some(base))
                .with_extension("filesnap");
            std::fs::create_dir_all(record.parent().context("Snapshot has no parent")?)?;
            let temporary = record.with_extension(format!("{}.tmp", captured.id));
            std::fs::write(&temporary, captured.id.to_string())?;
            std::fs::rename(temporary, record)?;
            Ok(())
        })
        .await??;
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
            if (filename.ends_with(".snap") || filename.ends_with(".filesnap"))
                && (latest_filename.is_none() || filename > latest_filename.clone().unwrap())
            {
                latest_filename = Some(filename);
                latest_path = Some(entry.path());
            }
        }

        Ok(latest_path)
    }

    /// Restore the newest per-file checkpoint, consuming it only on success.
    /// Legacy raw `.snap` files remain readable during migration.
    ///
    /// # Arguments
    /// * `path` - Local file whose newest checkpoint should be restored.
    ///
    /// # Errors
    /// Returns an error if no checkpoint exists, restoration fails, or its
    /// reference is invalid.
    pub async fn undo_snapshot(&self, path: PathBuf) -> Result<()> {
        let _guard = self.operations.lock().await;
        let snapshot = Snapshot::create(path.clone())?;

        // All the snaps for `path` are stored in `snapshot.path_hash()`
        // directory.
        let snapshot_dir = self.snapshots_directory.join(snapshot.path_hash());

        // Check if the `snapshot_dir` exists
        if !ForgeFS::exists(&snapshot_dir) {
            return Err(anyhow::anyhow!("No snapshots found for {path:?}"));
        }

        let base = self.snapshots_directory.clone();
        let group = snapshot.path_hash();
        tokio::task::spawn_blocking(move || Self::cleanup_retired(&base, &group)).await?;

        // Retrieve the latest snapshot path
        let snapshot_path = Self::find_recent_snapshot(&snapshot_dir)
            .await?
            .context(format!("No valid snapshots found for {path:?}"))?;

        if snapshot_path
            .extension()
            .is_some_and(|extension| extension == "filesnap")
        {
            let turn = String::from_utf8(ForgeFS::read(&snapshot_path).await?)?;
            let base = self.snapshots_directory.clone();
            let source = PathBuf::from(&snapshot.path);
            tokio::task::spawn_blocking(move || -> Result<()> {
                let session = turn.trim();
                anyhow::ensure!(
                    uuid::Uuid::parse_str(session).is_ok(),
                    "Invalid snapshot reference"
                );
                let store = filesnap::WorkspaceStore::open(&base, &base)?;
                let target = store
                    .target_for_turn(turn.trim())?
                    .context("Missing filesnap checkpoint")?;
                let manifest = store.manifest(target.manifest_id())?;
                let key = filesnap::canonical_key(&source)
                    .to_string_lossy()
                    .into_owned();
                anyhow::ensure!(
                    manifest.entries.len() + manifest.absent.len() == 1
                        && (manifest.entries.contains_key(&key) || manifest.absent.contains(&key)),
                    "Snapshot reference does not belong to the requested file"
                );
                // Save a safety point before writes; this can be restored even
                // if the operation only partially succeeds.
                let outcome = store.restore_to(
                    session,
                    &target,
                    filesnap::RestoreKind::Rewind { undo_for: Some(session) },
                    [source],
                    &filesnap::Gitignore::empty(),
                )?;
                anyhow::ensure!(
                    outcome.stats.failed.is_empty(),
                    "File restore failed: {:?}",
                    outcome.stats.failed
                );
                Ok(())
            })
            .await??;
        } else {
            // Compatibility with snapshots created before the filesnap backend.
            let content = ForgeFS::read(&snapshot_path).await?;
            ForgeFS::write(&path, content).await?;
        }

        if snapshot_path
            .extension()
            .is_some_and(|extension| extension == "filesnap")
        {
            // Retire the host marker before deleting its engine session. A crash or
            // failed cleanup leaves a retryable marker, never a usable broken undo.
            let retired = snapshot_path.with_extension("filesnap-retired");
            tokio::fs::rename(&snapshot_path, retired).await?;
            let base = self.snapshots_directory.clone();
            let group = snapshot.path_hash();
            tokio::task::spawn_blocking(move || Self::cleanup_retired(&base, &group)).await?;
        } else {
            ForgeFS::remove_file(&snapshot_path).await?;
        }

        Ok(())
    }

    fn cleanup_retired(base: &std::path::Path, group: &str) {
        let result = (|| -> Result<()> {
            let directory = base.join(group);
            if !directory.exists() {
                return Ok(());
            }
            let store = filesnap::WorkspaceStore::open(base, base)?;
            for entry in std::fs::read_dir(directory)? {
                let path = entry?.path();
                if !path
                    .extension()
                    .is_some_and(|extension| extension == "filesnap-retired")
                {
                    continue;
                }
                let session = std::fs::read_to_string(&path)?.trim().to_owned();
                anyhow::ensure!(
                    uuid::Uuid::parse_str(&session).is_ok(),
                    "Invalid retired snapshot reference"
                );
                let outcome = store.delete_sessions(&[session]);
                anyhow::ensure!(
                    outcome.refused.is_empty() && outcome.incomplete.is_empty(),
                    "Snapshot cleanup is incomplete: {outcome:?}"
                );
                std::fs::remove_file(path)?;
            }
            filesnap::collect_garbage(base)?;
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!(%error, "Snapshot content cleanup deferred; retired references remain retryable");
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
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
            self.service.create_snapshot(self.test_file.clone()).await
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
    async fn test_absent_file_is_removed_without_touching_a_neighbor() {
        let fixture = TestContext::new().await.unwrap();
        let neighbor = fixture.test_file.with_file_name("neighbor.bin");
        ForgeFS::write(&neighbor, [0, 255, 7]).await.unwrap();
        fixture.create_snapshot().await.unwrap();
        fixture.write_content("created").await.unwrap();
        fixture.undo_snapshot().await.unwrap();
        let actual = (
            fixture.test_file.exists(),
            ForgeFS::read(neighbor).await.unwrap(),
        );
        let expected = (false, vec![0, 255, 7]);
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_legacy_snapshot_remains_restorable_after_a_new_snapshot() {
        let fixture = TestContext::new().await.unwrap();
        fixture.write_content("legacy").await.unwrap();
        let legacy = Snapshot::create(fixture.test_file.clone()).unwrap();
        let legacy_path = legacy.snapshot_path(Some(fixture.service.snapshots_directory.clone()));
        ForgeFS::create_dir_all(legacy_path.parent().unwrap())
            .await
            .unwrap();
        ForgeFS::write(legacy_path, "legacy").await.unwrap();
        fixture.write_content("new checkpoint").await.unwrap();
        fixture.create_snapshot().await.unwrap();
        fixture.write_content("latest").await.unwrap();
        fixture.undo_snapshot().await.unwrap();
        fixture.undo_snapshot().await.unwrap();
        let actual = fixture.read_content().await.unwrap();
        let expected = "legacy";
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_resume_and_failed_restore_keep_the_checkpoint_available() {
        let fixture = TestContext::new().await.unwrap();
        fixture.write_content("saved").await.unwrap();
        fixture.create_snapshot().await.unwrap();
        ForgeFS::remove_file(&fixture.test_file).await.unwrap();
        ForgeFS::create_dir_all(&fixture.test_file).await.unwrap();
        let resumed = SnapshotService::new(fixture.service.snapshots_directory.clone());
        let actual = resumed.undo_snapshot(fixture.test_file.clone()).await;
        assert!(actual.is_err());
        std::fs::remove_dir(&fixture.test_file).unwrap();
        resumed
            .undo_snapshot(fixture.test_file.clone())
            .await
            .unwrap();
        let actual = fixture.read_content().await.unwrap();
        let expected = "saved";
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn consumed_snapshot_releases_only_its_own_engine_session() -> Result<()> {
        let fixture = TestContext::new().await?;
        fixture.write_content("one").await?;
        let first = fixture.create_snapshot().await?;
        fixture.write_content("two").await?;
        let second = fixture.create_snapshot().await?;
        fixture.write_content("three").await?;
        fixture.undo_snapshot().await?;
        assert_eq!(fixture.read_content().await?, "two");
        let store = filesnap::WorkspaceStore::open(
            &fixture.service.snapshots_directory,
            &fixture.service.snapshots_directory,
        )?;
        assert_eq!(store.sessions()?, vec![first.id.to_string()]);
        assert!(store.target_for_turn(&second.id.to_string())?.is_none());
        fixture.undo_snapshot().await?;
        assert_eq!(fixture.read_content().await?, "one");
        assert!(store.sessions()?.is_empty());
        Ok(())
    }
}
