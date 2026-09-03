pub mod helios_agent;
pub mod hook;
pub mod manager;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use hook::{Hook, HookContext, HookResult};
pub use manager::{HeliosAgentPlugin, PluginManager, PluginRegistry};

/// Configuration for a plugin, typically loaded from a JSON file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    pub name: String,
    pub version: String,
    pub path: Option<String>, // Optional path for native Rust plugins
    pub enabled: bool,
}

/// The core Plugin trait that all plugins must implement.
#[async_trait]
pub trait Plugin: Send + Sync {
    /// The name of the plugin.
    fn name(&self) -> &str;

    /// The version of the plugin.
    fn version(&self) -> &str;

    /// Initialize the plugin.
    async fn init(&mut self) -> Result<()>;

    /// Shutdown the plugin gracefully.
    async fn shutdown(&mut self) -> Result<()>;
}

/// Metadata about a loaded plugin.
#[derive(Debug, Clone)]
pub struct PluginMetadata {
    pub name: String,
    pub version: String,
    pub enabled: bool,
}
