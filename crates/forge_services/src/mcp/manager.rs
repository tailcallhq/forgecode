use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use bytes::Bytes;
use forge_app::domain::{McpConfig, McpTrustStatus, McpTrustStore, Scope};
use forge_app::{
    EnvironmentInfra, FileInfoInfra, FileReaderInfra, FileWriterInfra, KVStore, McpConfigManager,
    McpServerInfra,
};
use merge::Merge;

pub struct ForgeMcpManager<I> {
    infra: Arc<I>,
}

impl<I> ForgeMcpManager<I> {
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra }
    }
}

impl<I> ForgeMcpManager<I>
where
    I: McpServerInfra
        + FileReaderInfra
        + FileInfoInfra
        + EnvironmentInfra
        + FileWriterInfra
        + KVStore,
{
    async fn read_config(&self, path: &Path) -> anyhow::Result<McpConfig> {
        let config = self.infra.read_utf8(path).await?;
        Ok(serde_json::from_str(&config)?)
    }

    async fn config_path(&self, scope: &Scope) -> anyhow::Result<PathBuf> {
        let env = self.infra.get_environment();
        match scope {
            Scope::User => Ok(env.mcp_user_config()),
            Scope::Local => Ok(env.mcp_local_config()),
        }
    }

    /// Reads the persisted trust store from disk, returning an empty store if
    /// the file is absent or its contents cannot be parsed.
    async fn read_trust_store(&self) -> anyhow::Result<McpTrustStore> {
        let path = self.infra.get_environment().mcp_trust_path();
        if !self.infra.is_file(&path).await.unwrap_or(false) {
            return Ok(McpTrustStore::default());
        }
        let content = self.infra.read_utf8(&path).await?;
        Ok(serde_json::from_str(&content).unwrap_or_default())
    }

    /// Writes the trust store to disk at the environment's `mcp_trust_path`.
    async fn write_trust_store(&self, store: &McpTrustStore) -> anyhow::Result<()> {
        let path = self.infra.get_environment().mcp_trust_path();
        let content = serde_json::to_string_pretty(store)?;
        self.infra.write(&path, Bytes::from(content)).await
    }

    /// Returns `true` if the project-local MCP config is either absent or has
    /// been explicitly trusted at its current content hash.
    async fn is_local_trusted(&self) -> anyhow::Result<bool> {
        let local_path = self.infra.get_environment().mcp_local_config();
        if !self.infra.is_file(&local_path).await.unwrap_or(false) {
            // No local config => nothing to gate.
            return Ok(true);
        }
        let hash = self.read_config(&local_path).await?.cache_key();
        let store = self.read_trust_store().await?;
        Ok(matches!(
            store.get_status(&local_path, hash),
            McpTrustStatus::Trusted
        ))
    }
}

#[async_trait::async_trait]
impl<I> McpConfigManager for ForgeMcpManager<I>
where
    I: McpServerInfra
        + FileReaderInfra
        + FileInfoInfra
        + EnvironmentInfra
        + FileWriterInfra
        + KVStore,
{
    async fn read_mcp_config(&self, scope: Option<&Scope>) -> anyhow::Result<McpConfig> {
        match scope {
            Some(scope) => {
                // Read only from the specified scope
                let config_path = self.config_path(scope).await?;
                if self.infra.is_file(&config_path).await.unwrap_or(false) {
                    self.read_config(&config_path).await
                } else {
                    Ok(McpConfig::default())
                }
            }
            None => {
                // Read and merge all configurations (original behavior)
                let env = self.infra.get_environment();
                let paths = vec![
                    // Configs at lower levels take precedence, so we read them in reverse order.
                    env.mcp_user_config().as_path().to_path_buf(),
                    env.mcp_local_config().as_path().to_path_buf(),
                ];
                let mut config = McpConfig::default();
                for path in paths {
                    if self.infra.is_file(&path).await.unwrap_or_default() {
                        let new_config = self.read_config(&path).await.context(format!(
                            "An error occurred while reading config at: {}",
                            path.display()
                        ))?;
                        config.merge(new_config);
                    }
                }
                Ok(config)
            }
        }
    }

    async fn write_mcp_config(&self, config: &McpConfig, scope: &Scope) -> anyhow::Result<()> {
        // Write config
        self.infra
            .write(
                self.config_path(scope).await?.as_path(),
                Bytes::from(serde_json::to_string_pretty(config)?),
            )
            .await?;

        // Clear the unified cache to force refresh on next use
        // Since we now use a merged hash, clearing any scope invalidates the cache
        self.infra.cache_clear().await?;

        Ok(())
    }

    async fn get_mcp_trust_status(&self, path: &Path) -> anyhow::Result<McpTrustStatus> {
        let hash = self.read_config(path).await?.cache_key();
        let store = self.read_trust_store().await?;
        Ok(store.get_status(path, hash))
    }

    async fn set_mcp_trust(&self, path: &Path, status: McpTrustStatus) -> anyhow::Result<()> {
        let hash = self.read_config(path).await?.cache_key();
        let mut store = self.read_trust_store().await?;

        match status {
            McpTrustStatus::Trusted => store.trust(path, hash),
            McpTrustStatus::Rejected => store.reject(path, hash),
            McpTrustStatus::Unknown => store.clear(path),
        }

        self.write_trust_store(&store).await
    }

    async fn filter_trusted(&self, raw: McpConfig) -> anyhow::Result<McpConfig> {
        if self.is_local_trusted().await? {
            return Ok(raw);
        }

        // Local is untrusted: drop any servers whose names appear only in the
        // local config, retaining those defined in the user scope.
        let user_config = self.read_mcp_config(Some(&Scope::User)).await?;
        let mut filtered = raw;
        filtered
            .mcp_servers
            .retain(|name, _| user_config.mcp_servers.contains_key(name));
        Ok(filtered)
    }
}
