use std::path::Path;

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt as _;

impl crate::ForgeFS {
    pub async fn create_dir_all<T: AsRef<Path>>(path: T) -> Result<()> {
        tokio::fs::create_dir_all(path.as_ref())
            .await
            .with_context(|| format!("Failed to create dir {}", path.as_ref().display()))
    }

    pub async fn write<T: AsRef<Path>, U: AsRef<[u8]>>(path: T, contents: U) -> Result<()> {
        tokio::fs::write(path.as_ref(), contents)
            .await
            .with_context(|| format!("Failed to write file {}", path.as_ref().display()))
    }

    /// Appends content to an existing file, or creates it if it does not exist.
    pub async fn append<T: AsRef<Path>, U: AsRef<[u8]>>(path: T, contents: U) -> Result<()> {
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path.as_ref())
            .await
            .with_context(|| {
                format!(
                    "Failed to open file for appending {}",
                    path.as_ref().display()
                )
            })?;
        file.write_all(contents.as_ref())
            .await
            .with_context(|| format!("Failed to append to file {}", path.as_ref().display()))
    }

    pub async fn remove_file<T: AsRef<Path>>(path: T) -> Result<()> {
        tokio::fs::remove_file(path.as_ref())
            .await
            .with_context(|| format!("Failed to remove file {}", path.as_ref().display()))
    }
}
