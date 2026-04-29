use std::sync::Arc;

use anyhow::Result;
use forge_domain::{AgentId, ModelId, Provider};

use crate::{AgentRegistry, AppConfigService, ProviderAuthService, ProviderService};

/// Resolver for agent providers and models.
/// Handles provider resolution, credential refresh, and model lookup.
pub struct AgentProviderResolver<S>(Arc<S>);

impl<S> AgentProviderResolver<S> {
    /// Creates a new AgentProviderResolver instance
    pub fn new(services: Arc<S>) -> Self {
        Self(services)
    }
}

impl<S> AgentProviderResolver<S>
where
    S: AgentRegistry + ProviderService + AppConfigService + ProviderAuthService,
{
    /// Gets the provider for the specified agent, or the default provider if no
    /// agent is provided. Automatically refreshes OAuth credentials if they're
    /// about to expire.
    pub async fn get_provider(&self, agent_id: Option<AgentId>) -> Result<Provider<url::Url>> {
        let provider_id = if let Some(agent_id) = agent_id {
            // Load all agent definitions and find the one we need

            if let Some(agent) = self.0.get_agent(&agent_id).await? {
                // If the agent definition has a provider, use it; otherwise use default
                agent.provider
            } else {
                // TODO: Needs review, should we throw an err here?
                // we can throw crate::Error::AgentNotFound
                self.0
                    .get_session_config()
                    .await
                    .map(|c| c.provider)
                    .ok_or_else(|| forge_domain::Error::NoDefaultSession)?
            }
        } else {
            self.0
                .get_session_config()
                .await
                .map(|c| c.provider)
                .ok_or_else(|| forge_domain::Error::NoDefaultSession)?
        };

        let provider = self.0.get_provider(provider_id).await?;
        Ok(provider)
    }

    /// Gets the model for the specified agent, or the default model if no agent
    /// is provided
    pub async fn get_model(&self, agent_id: Option<AgentId>) -> Result<ModelId> {
        if let Some(agent_id) = agent_id {
            if let Some(agent) = self.0.get_agent(&agent_id).await? {
                Ok(agent.model)
            } else {
                // TODO: Needs review, should we throw an err here?
                // we can throw crate::Error::AgentNotFound
                self.0
                    .get_session_config()
                    .await
                    .map(|c| c.model)
                    .ok_or_else(|| forge_domain::Error::NoDefaultSession.into())
            }
        } else {
            self.0
                .get_session_config()
                .await
                .map(|c| c.model)
                .ok_or_else(|| forge_domain::Error::NoDefaultSession.into())
        }
    }
}
