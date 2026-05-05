use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use bytes::Bytes;
use forge_app::domain::{McpConfig, McpServerConfig, McpTrustResponse, McpTrustStore, Scope};
use forge_app::{
    EnvironmentInfra, FileInfoInfra, FileReaderInfra, FileWriterInfra, KVStore, McpConfigManager,
    McpServerInfra, UserInfra,
};
use merge::Merge;

pub struct ForgeMcpManager<I> {
    infra: Arc<I>,
    session_trusted: Mutex<HashSet<(String, u64)>>,
}

impl<I> ForgeMcpManager<I>
where
    I: McpServerInfra + FileReaderInfra + FileInfoInfra + EnvironmentInfra + KVStore,
{
    /// Creates a new [`ForgeMcpManager`] wrapping the provided infrastructure.
    pub fn new(infra: Arc<I>) -> Self {
        Self { infra, session_trusted: Default::default() }
    }

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
}

impl<I> ForgeMcpManager<I>
where
    I: McpServerInfra
        + FileReaderInfra
        + FileInfoInfra
        + EnvironmentInfra
        + FileWriterInfra
        + KVStore
        + UserInfra,
{
    /// Reads the persisted trust store from disk, returning a default empty
    /// store if the file is absent.
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

    /// Returns true if the given path+hash pair is trusted for this session.
    fn is_session_trusted(&self, path: &Path, hash: u64) -> bool {
        self.session_trusted
            .lock()
            .unwrap()
            .contains(&(path.to_string_lossy().into_owned(), hash))
    }

    /// Records a session-scoped trust decision.
    fn add_session_trust(&self, path: &Path, hash: u64) {
        self.session_trusted
            .lock()
            .unwrap()
            .insert((path.to_string_lossy().into_owned(), hash));
    }

    /// Applies the interactive trust gate for a project-local MCP config.
    ///
    /// Returns the config as-is when already trusted (session or persistent),
    /// otherwise prompts the user and acts on their choice.
    async fn apply_trust_gate(
        &self,
        local: McpConfig,
        local_path: &Path,
    ) -> anyhow::Result<McpConfig> {
        if local.mcp_servers.is_empty() {
            return Ok(local);
        }

        let hash = local.cache_key();

        // Fast path: already trusted in this session.
        if self.is_session_trusted(local_path, hash) {
            return Ok(local);
        }

        // Check persistent trust store.
        let mut store = self.read_trust_store().await?;
        if store.is_trusted(local_path, hash) {
            self.add_session_trust(local_path, hash);
            return Ok(local);
        }

        // Build and display the prompt.
        let prompt = format_trust_prompt(local_path, &local);
        match self
            .infra
            .select_one_enum::<McpTrustResponse>(&prompt)
            .await?
        {
            Some(McpTrustResponse::TrustAndRemember) => {
                store.remember(local_path.to_path_buf(), hash);
                self.write_trust_store(&store).await?;
                self.add_session_trust(local_path, hash);
                Ok(local)
            }
            Some(McpTrustResponse::TrustOnce) => {
                self.add_session_trust(local_path, hash);
                Ok(local)
            }
            Some(McpTrustResponse::Reject) | None => Ok(McpConfig::default()),
        }
    }
}

/// Builds the interactive prompt string shown to the user when an untrusted
/// project-local `.mcp.json` is found. Lists the file path and every server
/// name together with its command or URL.
fn format_trust_prompt(path: &Path, config: &McpConfig) -> String {
    let mut lines = vec![format!(
        "A project-local MCP config was found at {}.\nThe following servers would be started:",
        path.display()
    )];
    for (name, server) in &config.mcp_servers {
        let detail = match server {
            McpServerConfig::Stdio(s) => format!("command: {}", s.command),
            McpServerConfig::Http(s) => format!("url: {}", s.url),
        };
        lines.push(format!("  • {name} ({detail})"));
    }
    lines.push("\nDo you trust these servers?".to_string());
    lines.join("\n")
}

#[async_trait::async_trait]
impl<I> McpConfigManager for ForgeMcpManager<I>
where
    I: McpServerInfra
        + FileReaderInfra
        + FileInfoInfra
        + EnvironmentInfra
        + FileWriterInfra
        + KVStore
        + UserInfra,
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

    async fn filter_trusted(&self, raw: McpConfig) -> anyhow::Result<McpConfig> {
        let env = self.infra.get_environment();

        // User-scope config is always implicitly trusted.
        let user_path = env.mcp_user_config();
        let user_config = if self.infra.is_file(&user_path).await.unwrap_or(false) {
            self.read_config(&user_path).await?
        } else {
            McpConfig::default()
        };

        // Local-scope config must pass the trust gate.
        let local_path = env.mcp_local_config();
        let local_config = if self.infra.is_file(&local_path).await.unwrap_or(false) {
            let local_raw = self.read_config(&local_path).await?;
            self.apply_trust_gate(local_raw, &local_path).await?
        } else {
            McpConfig::default()
        };

        // Merge: user first, then local (local takes precedence as in read_mcp_config).
        let mut merged = user_config;
        merged.merge(local_config);

        // Retain only servers that exist in the merged trusted set.
        let trusted_keys: std::collections::BTreeSet<_> = merged.mcp_servers.keys().cloned().collect();
        let mut result = raw;
        result.mcp_servers.retain(|k, _| trusted_keys.contains(k));
        Ok(result)
    }
}
