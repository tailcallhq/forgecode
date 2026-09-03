use async_trait::async_trait;
use forge_domain::{Agent, Conversation, Environment, EventData, EventHandle, ResponsePayload};
use tracing::{debug, info};

use crate::compact::Compactor;
use crate::{compression, prune};

/// Hook handler that performs context compaction when needed
///
/// This handler checks if the conversation context has grown too large
/// and compacts it according to the agent's compaction configuration.
/// The handler mutates the conversation's context in-place if compaction
/// is triggered.
///
/// It also applies programmatic/semantic/AI-based compression and pruning
/// hooks before and after the standard compaction step, enabling the
/// heliosLite fork to do more than just token-count-based eviction.
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
            let token_count = context.token_count_approx();

            // Phase 1: AI-driven semantic compression (before standard compaction)
            let compressed = if self.agent.compact.enable_semantic_compression {
                info!(agent_id = %self.agent.id, "Semantic compression phase");
                compression::compress(context.clone(), &self.agent.compact).0
            } else {
                context.clone()
            };

            // Phase 2: AI-driven importance pruning (before standard compaction)
            let pruned = if self.agent.compact.enable_structural_dedup {
                info!(agent_id = %self.agent.id, "Structural dedup / importance pruning phase");
                prune::prune(&compressed, &self.agent.compact).0
            } else {
                compressed
            };

            // Phase 3: Standard compaction (token-count-based)
            if self.agent.compact.should_compact(&pruned, token_count) {
                info!(agent_id = %self.agent.id, "Compaction triggered by hook");
                let compacted =
                    Compactor::new(self.agent.compact.clone(), self.environment.clone())
                        .compact(pruned, false)?;
                conversation.context = Some(compacted);
            } else {
                debug!(agent_id = %self.agent.id, "Compaction not needed");
                // Still apply the pruned/compressed version even if standard compaction isn't
                // triggered
                conversation.context = Some(pruned);
            }
        }
        Ok(())
    }
}
