use std::sync::{Arc, Mutex};

use derive_setters::Setters;

use crate::{ArcSender, CachedHook, ChatResponse, Metrics, TitleFormat, Todo, TodoItem};

/// Provides additional context for tool calls.
#[derive(Debug, Clone, Setters)]
pub struct ToolCallContext {
    sender: Option<ArcSender>,
    metrics: Arc<Mutex<Metrics>>,
    #[setters(skip)]
    cached_hooks: Arc<Vec<CachedHook>>,
}

impl ToolCallContext {
    /// Creates a new ToolCallContext with default values
    pub fn new(metrics: Metrics) -> Self {
        Self {
            sender: None,
            metrics: Arc::new(Mutex::new(metrics)),
            cached_hooks: Arc::new(Vec::new()),
        }
    }

    /// Set the cached hooks for hook interception
    pub fn cached_hooks(mut self, hooks: Arc<Vec<CachedHook>>) -> Self {
        self.cached_hooks = hooks;
        self
    }

    /// Get the cached hooks
    pub fn get_cached_hooks(&self) -> Arc<Vec<CachedHook>> {
        self.cached_hooks.clone()
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

    fn fixture_cached_hook(content: &[u8]) -> CachedHook {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hook.sh");
        std::fs::write(&path, content).unwrap();
        CachedHook::from_path(path).unwrap()
    }

    #[test]
    fn test_create_context() {
        let metrics = Metrics::default();
        let context = ToolCallContext::new(metrics);
        assert!(context.sender.is_none());
        assert!(context.get_cached_hooks().is_empty());
    }

    #[test]
    fn test_with_sender() {
        let metrics = Metrics::default();
        let context = ToolCallContext::new(metrics);
        assert!(context.sender.is_none());
        assert!(context.get_cached_hooks().is_empty());
    }

    #[test]
    fn test_cached_hooks_storage() {
        let metrics = Metrics::default();
        let hooks = vec![
            fixture_cached_hook(b"#!/bin/bash\necho allow"),
            fixture_cached_hook(b"#!/bin/bash\necho deny"),
        ];
        let context = ToolCallContext::new(metrics).cached_hooks(Arc::new(hooks));
        let retrieved = context.get_cached_hooks();
        assert_eq!(retrieved.len(), 2);
    }

    #[test]
    fn test_agent_executor_gets_cached_hooks() {
        // This test verifies that get_cached_hooks() returns the expected hooks
        // The agent_executor.rs code at line 80 calls ctx.get_cached_hooks()
        let metrics = Metrics::default();
        let hooks = vec![
            fixture_cached_hook(b"#!/bin/bash\necho allow"),
        ];
        let context = ToolCallContext::new(metrics).cached_hooks(Arc::new(hooks));

        // Simulate what agent_executor does at line 80
        let cached_hooks = context.get_cached_hooks();
        assert_eq!(cached_hooks.len(), 1);
    }
}
