use std::sync::{Arc, Mutex};

use derive_setters::Setters;

use crate::{ArcSender, ChatResponse, ConversationId, Metrics, TitleFormat, Todo, TodoItem};

/// Provides additional context for tool calls.
#[derive(Debug, Clone, Setters)]
pub struct ToolCallContext {
    sender: Option<ArcSender>,
    metrics: Arc<Mutex<Metrics>>,
    conversation_id: ConversationId,
}

impl ToolCallContext {
    /// Creates a new ToolCallContext with default values
    pub fn new(metrics: Metrics) -> Self {
        Self {
            sender: None,
            metrics: Arc::new(Mutex::new(metrics)),
            conversation_id: ConversationId::default(),
        }
    }

    /// Gets the conversation ID for this tool execution context
    pub fn get_conversation_id(&self) -> ConversationId {
        self.conversation_id
    }

    /// Send a message through the sender if available
    pub async fn send(&self, agent_message: impl Into<ChatResponse>) -> anyhow::Result<()> {
        if let Some(sender) = &self.sender {
            sender.send(Ok(agent_message.into())).await?
        }
        Ok(())
    }

    /// Send tool input title - MUST ONLY be used for presenting tool input
    /// information
    pub async fn send_tool_input(&self, title: impl Into<TitleFormat>) -> anyhow::Result<()> {
        let title = title.into();
        self.send(ChatResponse::TaskMessage {
            content: crate::ChatResponseContent::ToolInput(title),
        })
        .await
    }

    /// Execute a closure with access to the metrics
    pub fn with_metrics<F, R>(&self, f: F) -> anyhow::Result<R>
    where
        F: FnOnce(&mut Metrics) -> R,
    {
        let mut metrics = self
            .metrics
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire metrics lock"))?;
        Ok(f(&mut metrics))
    }

    /// Execute a fallible closure with access to the metrics
    pub fn try_with_metrics<F, R>(&self, f: F) -> anyhow::Result<R>
    where
        F: FnOnce(&mut Metrics) -> anyhow::Result<R>,
    {
        let mut metrics = self
            .metrics
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire metrics lock"))?;
        f(&mut metrics)
    }

    /// Returns all known todos (active and historical completed todos).
    ///
    /// # Errors
    ///
    /// Returns an error if the metrics lock cannot be acquired.
    pub fn get_todos(&self) -> anyhow::Result<Vec<Todo>> {
        self.with_metrics(|metrics| metrics.get_todos().to_vec())
    }

    /// Applies incremental todo changes using content as the matching key.
    ///
    /// # Arguments
    ///
    /// * `changes` - Todo items to add, update, or remove (via `cancelled`
    ///   status).
    ///
    /// # Errors
    ///
    /// Returns an error if the metrics lock cannot be acquired or todo
    /// validation fails.
    pub fn update_todos(&self, changes: Vec<TodoItem>) -> anyhow::Result<Vec<Todo>> {
        self.try_with_metrics(|metrics| metrics.apply_todo_changes(changes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_context() {
        let metrics = Metrics::default();
        let context = ToolCallContext::new(metrics);
        assert!(context.sender.is_none());
    }

    #[test]
    fn test_with_sender() {
        let metrics = Metrics::default();
        let context = ToolCallContext::new(metrics);
        assert!(context.sender.is_none());
    }
}
