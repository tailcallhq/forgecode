use derive_setters::Setters;
use forge_domain::{McpServers, ToolDefinition};
use serde::{Deserialize, Serialize};

/// A comprehensive view of all tools available in the environment,
/// categorized by their source type for easier navigation and understanding.
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Setters)]
#[setters(into, strip_option)]
pub struct ToolsOverview {
    /// System tools provided by the Forge environment
    pub system: Vec<ToolDefinition>,
    /// Tools provided by registered agents
    pub agents: Vec<ToolDefinition>,
    /// Tools provided by MCP servers, grouped by server name
    pub mcp: McpServers,
}

impl ToolsOverview {
    /// Create a new empty ToolsOverview
    pub fn new() -> Self {
        ToolsOverview::default()
    }

    // Creates a flat list of all tool definitions
    pub fn as_vec(&self) -> Vec<&ToolDefinition> {
        let mut tools = Vec::new();
        tools.extend(&self.system);
        tools.extend(&self.agents);

        // Sort MCP servers by name for stable tool order (implicit caching)
        let mut server_names: Vec<_> = self.mcp.get_servers().keys().collect();
        server_names.sort();

        for server_name in server_names {
            if let Some(server_tools) = self.mcp.get_servers().get(server_name) {
                tools.extend(server_tools);
            }
        }
        tools
    }
}

impl From<ToolsOverview> for Vec<ToolDefinition> {
    fn from(value: ToolsOverview) -> Self {
        value.as_vec().into_iter().cloned().collect()
    }
}
