use std::sync::{Arc, Mutex};

use derive_setters::Setters;

use crate::{ArcSender, ChatResponse, ConversationId, Metrics, TitleFormat, Todo, TodoItem};

/// Provides additional context for tool calls.
#[derive(Debug, Clone, Setters)]
pub struct ToolCallContext {
    sender: Option<ArcSender>,
    metrics: Arc<Mutex<Metrics>>,
    #[setters(skip)]
    conversation_id: Option<ConversationId>,
    #[setters(skip)]
    parent_id: Option<ConversationId>,
    #[setters(skip)]
    source: Option<String>,
}

impl ToolCallContext {
    /// Creates a new ToolCallContext with default values
    pub fn new(metrics: Metrics) -> Self {
        Self { sender: None, metrics: Arc::new(Mutex::new(metrics)), conversation_id: None, parent_id: None, source: None }
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

    /// Returns the conversation ID associated with this tool call context, if any.
    pub fn conversation_id(&self) -> Option<ConversationId> {
        self.conversation_id
    }

    /// Sets the conversation ID for this tool call context.
    pub fn set_conversation_id(&mut self, id: Option<ConversationId>) {
        self.conversation_id = id;
    }

    /// Returns the parent conversation ID associated with this tool call context, if any.
    pub fn parent_id(&self) -> Option<ConversationId> {
        self.parent_id
    }

    /// Sets the parent conversation ID for this tool call context.
    pub fn set_parent_id(&mut self, id: Option<ConversationId>) {
        self.parent_id = id;
    }

    /// Returns the source associated with this tool call context, if any.
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    /// Sets the source for this tool call context.
    pub fn set_source(&mut self, source: Option<String>) {
        self.source = source;
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
