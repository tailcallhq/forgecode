use std::sync::Arc;

use anyhow::Result;
use forge_domain::*;
use merge::Merge;

use crate::services::{McpConfigManager, McpService, PolicyService};
use crate::{EnvironmentInfra, Services};

/// McpApp handles MCP permission reconciliation and policy-filtered
/// connections, keeping `McpService` free of any policy awareness.
pub struct McpApp<S> {
    services: Arc<S>,
}

impl<S> McpApp<S> {
    /// Creates a new McpApp instance with the provided services.
    pub fn new(services: Arc<S>) -> Self {
        Self { services }
    }
}

impl<S: Services + EnvironmentInfra<Config = forge_config::ForgeConfig>> McpApp<S> {
    /// Prompts for missing permissions for each enabled server in `cfg`.
    /// Idempotent — servers that already have a recorded decision are skipped
    /// silently.
    ///
    /// This is the only place a permission prompt can fire for MCP. Call this
    /// synchronously at startup (before the REPL takes over stdin) so prompts
    /// don't race with user input.
    pub async fn request_mcp_permissions(&self, cfg: McpConfig) -> Result<()> {
        let cwd = self.services.get_environment().cwd;
        for (name, server) in cfg
            .mcp_servers
            .into_iter()
            .filter(|(_, s)| !s.is_disabled())
        {
            let op = PermissionOperation::Mcp {
                config: server,
                cwd: cwd.clone(),
                message: format!("Allow MCP server \"{name}\" to connect?"),
            };
            // check_operation_permission handles the prompt + persist.
            // The return value is intentionally discarded here; the caller
            // just needs all decisions to be recorded before connections start.
            let _ = self.services.check_operation_permission(&op).await?;
        }
        Ok(())
    }

    /// Returns a merged MCP config where user-scope servers are trusted
    /// unconditionally and local-scope servers are filtered to those with an
    /// explicit `Allow` policy. Never prompts — call
    /// [`Self::request_mcp_permissions`] first to ensure decisions exist.
    pub async fn permitted_mcp_config(&self) -> Result<McpConfig> {
        let mut user = self.services.read_mcp_config(Some(&Scope::User)).await?;
        let local = self.services.read_mcp_config(Some(&Scope::Local)).await?;
        let cwd = self.services.get_environment().cwd;

        let mut filtered_local = McpConfig::default();
        for (name, server) in local.mcp_servers {
            if server.is_disabled() {
                continue;
            }
            let op = PermissionOperation::Mcp {
                config: server.clone(),
                cwd: cwd.clone(),
                message: String::new(),
            };
            if self.services.is_operation_permitted(&op).await? {
                filtered_local.mcp_servers.insert(name, server);
            }
        }
        user.merge(filtered_local);
        Ok(user)
    }

    /// Lists MCP tools, connecting only servers that have an explicit `Allow`
    /// policy for local-scope entries (user-scope are trusted unconditionally).
    /// Never prompts.
    pub async fn get_mcp_servers(&self) -> Result<McpServers> {
        let cfg = self.permitted_mcp_config().await?;
        self.services.get_mcp_servers(cfg).await
    }

    /// Persist `Allow` decisions for the named servers without prompting.
    /// Used by `mcp import` to record consent on the user's behalf — importing
    /// is itself an explicit opt-in.
    pub async fn allow_mcp_servers(&self, names: &[ServerName]) -> Result<()> {
        let cfg = self.services.read_mcp_config(None).await?;
        let cwd = self.services.get_environment().cwd;
        for name in names {
            if let Some(server) = cfg.mcp_servers.get(name) {
                let op = PermissionOperation::Mcp {
                    config: server.clone(),
                    cwd: cwd.clone(),
                    message: format!("Connect to MCP server: {name}"),
                };
                self.services.allow_operation(&op).await?;
            }
        }
        Ok(())
    }
}
