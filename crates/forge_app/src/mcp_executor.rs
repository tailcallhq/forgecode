use std::sync::Arc;

use forge_domain::{TitleFormat, ToolCallContext, ToolCallFull, ToolName, ToolOutput};

use crate::{EnvironmentInfra, McpApp, McpService, Services};

pub struct McpExecutor<S> {
    services: Arc<S>,
    /// Shared `McpApp` instance so `permitted_mcp_config` is computed at most
    /// once per executor lifetime rather than on every tool call.
    mcp_app: McpApp<S>,
}

impl<S: Services + EnvironmentInfra<Config = forge_config::ForgeConfig>> McpExecutor<S> {
    pub fn new(services: Arc<S>) -> Self {
        let mcp_app = McpApp::new(services.clone());
        Self { services, mcp_app }
    }

    pub async fn execute(
        &self,
        input: ToolCallFull,
        context: &ToolCallContext,
    ) -> anyhow::Result<ToolOutput> {
        context
            .send_tool_input(TitleFormat::info("MCP").sub_title(input.name.as_str()))
            .await?;

        let cfg = self.mcp_app.permitted_mcp_config().await?;
        self.services.execute_mcp(input, cfg).await
    }

    pub async fn contains_tool(&self, tool_name: &ToolName) -> anyhow::Result<bool> {
        let mcp_servers = self.mcp_app.get_mcp_servers().await?;
        // Convert Claude Code format (mcp__{server}__{tool}) to the internal legacy
        // format (mcp_{server}_tool_{tool}) before checking, so both name styles match.
        let legacy = tool_name.to_legacy_mcp_name();
        let found = mcp_servers.get_servers().values().any(|tools| {
            tools
                .iter()
                .any(|tool| tool.name == *tool_name || legacy.as_ref() == Some(&tool.name))
        });
        Ok(found)
    }
}
