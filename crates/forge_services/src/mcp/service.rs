use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use forge_app::domain::{
    McpConfig, McpServerConfig, McpServers, ServerName, ToolCallFull, ToolDefinition, ToolName,
    ToolOutput,
};
use forge_app::{
    EnvironmentInfra, KVStore, McpClientInfra, McpConfigManager, McpServerInfra, McpService,
};
use tokio::sync::{Mutex, RwLock};

use crate::mcp::tool::McpExecutor;

fn generate_mcp_tool_name(server_name: &ServerName, tool_name: &ToolName) -> ToolName {
    let sanitized_server_name = ToolName::sanitized(server_name.to_string().as_str());
    let sanitized_tool_name = tool_name.clone().into_sanitized();

    ToolName::new(format!(
        "mcp_{sanitized_server_name}_tool_{sanitized_tool_name}"
    ))
}

#[derive(Clone)]
pub struct ForgeMcpService<M, I, C> {
    tools: Arc<RwLock<HashMap<ToolName, ToolHolder<McpExecutor<C>>>>>,
    failed_servers: Arc<RwLock<HashMap<ServerName, String>>>,
    previous_config_hash: Arc<Mutex<u64>>,
    manager: Arc<M>,
    infra: Arc<I>,
}

#[derive(Clone)]
struct ToolHolder<T> {
    definition: ToolDefinition,
    executable: T,
    server_name: String,
}

impl<M, I, C> ForgeMcpService<M, I, C>
where
    M: McpConfigManager,
    I: McpServerInfra + KVStore + EnvironmentInfra,
    C: McpClientInfra + Clone,
    C: From<<I as McpServerInfra>::Client>,
{
    pub fn new(manager: Arc<M>, infra: Arc<I>) -> Self {
        Self {
            tools: Default::default(),
            failed_servers: Default::default(),
            previous_config_hash: Arc::new(Mutex::new(Default::default())),
            manager,
            infra,
        }
    }

    async fn is_config_modified(&self, config: &McpConfig) -> bool {
        *self.previous_config_hash.lock().await != config.cache_key()
    }

    async fn insert_clients(&self, server_name: &ServerName, client: Arc<C>) -> anyhow::Result<()> {
        let tools = client.list().await?;

        let mut tool_map = self.tools.write().await;

        for mut tool in tools.into_iter() {
            let actual_name = tool.name.clone();
            let server = McpExecutor::new(actual_name, client.clone())?;
            let generated_name = generate_mcp_tool_name(server_name, &tool.name);

            tool.name = generated_name.clone();

            tool_map.insert(
                generated_name,
                ToolHolder {
                    definition: tool,
                    executable: server,
                    server_name: server_name.to_string(),
                },
            );
        }

        Ok(())
    }

    async fn connect(
        &self,
        server_name: &ServerName,
        config: McpServerConfig,
    ) -> anyhow::Result<()> {
        let env_vars = self.infra.get_env_vars();
        let environment = self.infra.get_environment();
        let client = self.infra.connect(config, &env_vars, &environment).await?;
        let client = Arc::new(C::from(client));
        self.insert_clients(server_name, client).await?;

        Ok(())
    }

    async fn init_mcp(&self) -> anyhow::Result<()> {
        let mcp = self.manager.read_mcp_config(None).await?;

        // If config is unchanged, skip reinitialization
        if !self.is_config_modified(&mcp).await {
            return Ok(());
        }

        self.update_mcp(mcp).await
    }

    async fn update_mcp(&self, mcp: McpConfig) -> Result<(), anyhow::Error> {
        // Update the hash with the new config
        let new_hash = mcp.cache_key();
        *self.previous_config_hash.lock().await = new_hash;
        self.clear_tools().await;

        // Clear failed servers map before attempting new connections
        self.failed_servers.write().await.clear();

        let connections: Vec<_> = mcp
            .mcp_servers
            .into_iter()
            .filter(|v| !v.1.is_disabled())
            .map(|(name, server)| async move {
                let conn = self
                    .connect(&name, server)
                    .await
                    .context(format!("Failed to initiate MCP server: {name}"));

                (name, conn)
            })
            .collect();

        let results = futures::future::join_all(connections).await;

        for (server_name, result) in results {
            match result {
                Ok(_) => {}
                Err(error) => {
                    // Format error with full chain for detailed diagnostics
                    // Using Debug formatting with alternate flag shows the full error chain
                    let error_string = format!("{error:?}");
                    self.failed_servers
                        .write()
                        .await
                        .insert(server_name.clone(), error_string.clone());
                }
            }
        }

        Ok(())
    }

    async fn list(&self) -> anyhow::Result<McpServers> {
        self.init_mcp().await?;

        let tools = self.tools.read().await;
        let mut grouped_tools = std::collections::HashMap::new();

        for tool in tools.values() {
            grouped_tools
                .entry(ServerName::from(tool.server_name.clone()))
                .or_insert_with(Vec::new)
                .push(tool.definition.clone());
        }

        let failures = self.failed_servers.read().await.clone();

        Ok(McpServers::new(grouped_tools, failures))
    }
    async fn clear_tools(&self) {
        self.tools.write().await.clear()
    }

    async fn call(&self, call: ToolCallFull) -> anyhow::Result<ToolOutput> {
        // Ensure MCP connections are initialized before calling tools
        self.init_mcp().await?;

        let tools = self.tools.read().await;

        // Try exact match first, then fall back to legacy-format lookup for
        // tool calls arriving in the Claude Code `mcp__{server}__{tool}` format.
        let tool = tools
            .get(&call.name)
            .or_else(|| call.name.to_legacy_mcp_name().and_then(|n| tools.get(&n)))
            .context("Tool not found")?;

        tool.executable.call_tool(call.arguments.parse()?).await
    }

    /// Refresh the MCP cache by clearing cached data.
    /// Does NOT eagerly connect to servers - connections happen lazily
    /// when list() or call() is invoked, avoiding interactive OAuth during
    /// reload.
    async fn refresh_cache(&self) -> anyhow::Result<()> {
        // Clear the infra cache and reset config hash to force re-init on next access
        self.infra.cache_clear().await?;
        *self.previous_config_hash.lock().await = Default::default();
        self.clear_tools().await;
        self.failed_servers.write().await.clear();
        Ok(())
    }
}

#[async_trait::async_trait]
impl<M: McpConfigManager, I: McpServerInfra + KVStore + EnvironmentInfra, C> McpService
    for ForgeMcpService<M, I, C>
where
    C: McpClientInfra + Clone,
    C: From<<I as McpServerInfra>::Client>,
{
    async fn get_mcp_servers(&self) -> anyhow::Result<McpServers> {
        // Read current configs to compute merged hash
        let mcp_config = self.manager.read_mcp_config(None).await?;

        // Compute unified hash from merged config
        let config_hash = mcp_config.cache_key();

        // Check if cache is valid (exists and not expired)
        // Cache is valid, retrieve it
        if let Some(cache) = self.infra.cache_get::<_, McpServers>(&config_hash).await? {
            return Ok(cache.clone());
        }

        let servers = self.list().await?;
        self.infra.cache_set(&config_hash, &servers).await?;
        Ok(servers)
    }

    async fn execute_mcp(&self, call: ToolCallFull) -> anyhow::Result<ToolOutput> {
        self.call(call).await
    }

    async fn reload_mcp(&self) -> anyhow::Result<()> {
        self.refresh_cache().await
    }
}

#[cfg(test)]
mod tests {
    use forge_app::domain::{ServerName, ToolName};
    use pretty_assertions::assert_eq;

    use super::generate_mcp_tool_name;

    #[test]
    fn test_generate_mcp_tool_name_uses_legacy_format() {
        let fixture = ServerName::from("hugging-face".to_string());
        let actual = generate_mcp_tool_name(&fixture, &ToolName::new("read-channel"));
        let expected = ToolName::new("mcp_hugging_face_tool_read_channel");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_generate_mcp_tool_name_sanitizes_server_and_tool_names() {
        let fixture = ServerName::from("claude.ai Slack".to_string());
        let actual = generate_mcp_tool_name(&fixture, &ToolName::new("Add comment"));
        let expected = ToolName::new("mcp_claude_ai_slack_tool_add_comment");
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_to_legacy_mcp_name_converts_claude_code_format() {
        let actual = ToolName::new("mcp__github__create_issue").to_legacy_mcp_name();
        let expected = Some(ToolName::new("mcp_github_tool_create_issue"));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_to_legacy_mcp_name_converts_multipart_server_name() {
        let actual = ToolName::new("mcp__hugging_face__read_channel").to_legacy_mcp_name();
        let expected = Some(ToolName::new("mcp_hugging_face_tool_read_channel"));
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_to_legacy_mcp_name_returns_none_for_non_mcp_tools() {
        let actual = ToolName::new("read").to_legacy_mcp_name();
        assert_eq!(actual, None);
    }

    #[test]
    fn test_to_legacy_mcp_name_returns_none_for_legacy_format() {
        // Already in legacy format — should not double-convert
        let actual = ToolName::new("mcp_github_tool_create_issue").to_legacy_mcp_name();
        assert_eq!(actual, None);
    }
}
