use async_trait::async_trait;
use forge_domain::{Agent, Conversation, Environment, EventData, EventHandle, ResponsePayload};
use tracing::{debug, info};

use crate::compact::Compactor;

/// Hook handler that performs context compaction when needed
///
/// This handler checks if the conversation context has grown too large
/// and compacts it according to the agent's compaction configuration.
/// The handler mutates the conversation's context in-place if compaction
/// is triggered.
#[derive(Clone)]
pub struct CompactionHandler {
    agent: Agent,
    environment: Environment,
}

impl CompactionHandler {
    /// Creates a new compaction handler
    ///
    /// # Arguments
    /// * `agent` - The agent configuration containing compaction settings
    /// * `environment` - The environment configuration
    pub fn new(agent: Agent, environment: Environment) -> Self {
        Self { agent, environment }
    }
}

#[async_trait]
impl EventHandle<EventData<ResponsePayload>> for CompactionHandler {
    async fn handle(
        &self,
        _event: &EventData<ResponsePayload>,
        conversation: &mut Conversation,
    ) -> anyhow::Result<()> {
        if let Some(context) = &conversation.context {
            let token_count = context.token_count();
            if self.agent.compact.should_compact(context, *token_count) {
                info!(agent_id = %self.agent.id, "Compaction triggered by hook");
                let compacted =
                    Compactor::new(self.agent.compact.clone(), self.environment.clone())
                        .compact(context.clone(), false)?;
                conversation.context = Some(compacted);
            } else {
                debug!(agent_id = %self.agent.id, "Compaction not needed");
            }
        }
        Ok(())
    }
}
