use std::path::Path;

use anyhow::{Context, Result};
use bstr::ByteSlice;

impl crate::ForgeFS {
    pub async fn read_utf8<T: AsRef<Path>>(path: T) -> Result<String> {
        Self::read(path)
            .await
            .map(|bytes| bytes.to_str_lossy().to_string())
    }

    pub async fn read<T: AsRef<Path>>(path: T) -> Result<Vec<u8>> {
        tokio::fs::read(path.as_ref())
            .await
            .with_context(|| format!("Failed to read file {}", path.as_ref().display()))
    }

    pub async fn read_to_string<T: AsRef<Path>>(path: T) -> Result<String> {
        tokio::fs::read_to_string(path.as_ref())
            .await
            .with_context(|| format!("Failed to read file as string {}", path.as_ref().display()))
    }
}
