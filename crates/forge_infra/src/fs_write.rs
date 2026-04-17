use std::path::{Path, PathBuf};

use bytes::Bytes;
use forge_app::FileWriterInfra;

/// Low-level file write service
///
/// Provides primitive file write operations without snapshot coordination.
/// Snapshot management should be handled at the service layer.
pub struct ForgeFileWriteService;

impl ForgeFileWriteService {
    pub fn new() -> Self {
        Self
    }

    /// Creates parent directories for the given file path if they don't exist
    async fn create_parent_dirs(&self, path: &Path) -> anyhow::Result<()> {
        if !forge_fs::ForgeFS::exists(path)
            && let Some(parent) = path.parent()
        {
            forge_fs::ForgeFS::create_dir_all(parent).await?;
        }
        Ok(())
    }
}

impl Default for ForgeFileWriteService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl FileWriterInfra for ForgeFileWriteService {
    async fn write(&self, path: &Path, contents: Bytes) -> anyhow::Result<()> {
        self.create_parent_dirs(path).await?;
        Ok(forge_fs::ForgeFS::write(path, contents.to_vec()).await?)
    }

    async fn append(&self, path: &Path, contents: Bytes) -> anyhow::Result<()> {
        self.create_parent_dirs(path).await?;
        Ok(forge_fs::ForgeFS::append(path, contents.to_vec()).await?)
    }

    async fn write_temp(&self, prefix: &str, ext: &str, content: &str) -> anyhow::Result<PathBuf> {
        let path = tempfile::Builder::new()
            .disable_cleanup(true)
            .prefix(prefix)
            .suffix(ext)
            .tempfile()?
            .into_temp_path()
            .to_path_buf();

        self.create_parent_dirs(&path).await?;
        self.write(&path, content.to_string().into()).await?;

        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    fn create_test_service() -> ForgeFileWriteService {
        ForgeFileWriteService::new()
    }

    #[tokio::test]
    async fn test_create_parent_dirs_when_file_does_not_exist() {
        let temp_dir = tempdir().unwrap();
        let service = create_test_service();

        let nested_file_path = temp_dir
            .path()
            .join("level1")
            .join("level2")
            .join("test.txt");

        let actual = service
            .write(&nested_file_path, Bytes::from_static("foo".as_bytes()))
            .await;

        assert!(actual.is_ok());
        assert!(nested_file_path.parent().unwrap().exists());
    }
}
