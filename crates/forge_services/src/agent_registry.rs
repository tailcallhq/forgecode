use std::sync::Arc;

use dashmap::DashMap;
use forge_app::domain::AgentId;
use forge_app::{AgentRepository, EnvironmentInfra};
use forge_domain::{Agent, AgentInfo};
use tokio::sync::RwLock;

/// AgentRegistryService manages the active-agent ID and a registry of runtime
/// Agents in-memory. It lazily loads agents from AgentRepository on first
/// access.
pub struct ForgeAgentRegistryService<R> {
    // Infrastructure dependency for loading agents
    repository: Arc<R>,

    // In-memory storage for agents keyed by AgentId string
    // Lazily initialized on first access
    // Wrapped in RwLock to allow invalidation
    agents: RwLock<Option<DashMap<String, Agent>>>,

    // In-memory storage for the active agent ID
    active_agent_id: RwLock<Option<AgentId>>,
}

impl<R> ForgeAgentRegistryService<R> {
    /// Creates a new AgentRegistryService with the given repository
    pub fn new(repository: Arc<R>) -> Self {
        Self {
            repository,
            agents: RwLock::new(None),
            active_agent_id: RwLock::new(None),
        }
    }
}

impl<R: AgentRepository + EnvironmentInfra<Config = forge_config::ForgeConfig>>
    ForgeAgentRegistryService<R>
{
    /// Lazily initializes and returns the agents map
    /// Loads agents from repository on first call, subsequent calls return
    /// cached value
    async fn ensure_agents_loaded(&self) -> anyhow::Result<DashMap<String, Agent>> {
        // Check if already loaded
        {
            let agents_read = self.agents.read().await;
            if let Some(agents) = agents_read.as_ref() {
                return Ok(agents.clone());
            }
        }

        // Not loaded yet, acquire write lock and load
        let mut agents_write = self.agents.write().await;

        // Double-check in case another task loaded while we were waiting for write
        // lock
        if let Some(agents) = agents_write.as_ref() {
            return Ok(agents.clone());
        }

        // Load agents
        let agents_map = self.load_agents().await?;

        // Store and return
        *agents_write = Some(agents_map.clone());
        Ok(agents_map)
    }

    /// Load agents from repository and populate the in-memory map.
    ///
    /// Reads the default provider and model from the latest [`ForgeConfig`]
    /// (via `get_config()`) and passes them to the repository so agents that
    /// do not specify their own provider/model receive the session-level
    /// defaults.
    async fn load_agents(&self) -> anyhow::Result<DashMap<String, Agent>> {
        let agents = self.repository.get_agents().await?;
        let agents_map = DashMap::new();
        for agent in agents {
            agents_map.insert(agent.id.as_str().to_string(), agent);
        }

        Ok(agents_map)
    }
}

#[async_trait::async_trait]
impl<R: AgentRepository + EnvironmentInfra<Config = forge_config::ForgeConfig> + Send + Sync>
    forge_app::AgentRegistry for ForgeAgentRegistryService<R>
{
    async fn get_active_agent_id(&self) -> anyhow::Result<Option<AgentId>> {
        let agent_id = self.active_agent_id.read().await;
        Ok(agent_id.clone())
    }

    async fn set_active_agent_id(&self, agent_id: AgentId) -> anyhow::Result<()> {
        let mut active_agent = self.active_agent_id.write().await;
        *active_agent = Some(agent_id);
        Ok(())
    }

    async fn get_agents(&self) -> anyhow::Result<Vec<Agent>> {
        let agents = self.ensure_agents_loaded().await?;
        Ok(agents.iter().map(|entry| entry.value().clone()).collect())
    }

    async fn get_agent_infos(&self) -> anyhow::Result<Vec<AgentInfo>> {
        self.repository.get_agent_infos().await
    }

    async fn get_agent(&self, agent_id: &AgentId) -> anyhow::Result<Option<Agent>> {
        let agents = self.ensure_agents_loaded().await?;
        Ok(agents.get(agent_id.as_str()).map(|v| v.value().clone()))
    }

    async fn reload_agents(&self) -> anyhow::Result<()> {
        *self.agents.write().await = None;

        self.ensure_agents_loaded().await?;
        Ok(())
    }
}
