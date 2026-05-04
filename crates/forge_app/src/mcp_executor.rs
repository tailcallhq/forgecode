use std::sync::Arc;

use forge_domain::{TitleFormat, ToolCallContext, ToolCallFull, ToolName, ToolOutput};

use crate::McpService;

pub struct McpExecutor<S> {
    services: Arc<S>,
}

impl<S: McpService> McpExecutor<S> {
    pub fn new(services: Arc<S>) -> Self {
        Self { services }
    }

    pub async fn execute(
        &self,
        input: ToolCallFull,
        context: &ToolCallContext,
    ) -> anyhow::Result<ToolOutput> {
        context
            .send_tool_input(TitleFormat::info("MCP").sub_title(input.name.as_str()))
            .await?;

        self.services.execute_mcp(input).await
    }

    pub async fn contains_tool(&self, tool_name: &ToolName) -> anyhow::Result<bool> {
        let mcp_servers = self.services.get_mcp_servers().await?;
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
