use std::collections::{HashMap, hash_map};

use serde::{Deserialize, Serialize};

use crate::{ServerName, ToolDefinition};

/// Cache for MCP tool definitions
///
/// Simplified cache structure that stores only the essential data.
/// Validation and TTL checking are handled by the infrastructure layer
/// using cacache's built-in metadata capabilities.
#[derive(Default, Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpServers {
    /// Successfully loaded MCP servers with their tools
    servers: HashMap<ServerName, Vec<ToolDefinition>>,
    /// Failed MCP servers with their error messages
    #[serde(default)]
    failures: HashMap<ServerName, String>,
}

impl McpServers {
    /// Create a new cache entry with servers and failures
    pub fn new(
        servers: HashMap<ServerName, Vec<ToolDefinition>>,
        failures: HashMap<ServerName, String>,
    ) -> Self {
        Self { servers, failures }
    }

    /// Get the successful servers
    pub fn get_servers(&self) -> &HashMap<ServerName, Vec<ToolDefinition>> {
        &self.servers
    }

    /// Get the failed servers
    pub fn get_failures(&self) -> &HashMap<ServerName, String> {
        &self.failures
    }
}

impl IntoIterator for McpServers {
    type Item = (ServerName, Vec<ToolDefinition>);
    type IntoIter = hash_map::IntoIter<ServerName, Vec<ToolDefinition>>;

    fn into_iter(self) -> Self::IntoIter {
        self.servers.into_iter()
    }
}
