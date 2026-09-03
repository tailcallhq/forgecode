use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Context passed to hooks during tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    pub tool_name: String,
    pub input: serde_json::Value,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Result of a hook execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookResult {
    pub modified: bool,
    pub output: Option<serde_json::Value>,
    pub abort: bool,
    pub reason: Option<String>,
}

/// The Hook trait for intercepting tool calls.
#[async_trait]
pub trait Hook: Send + Sync {
    /// The name of the hook.
    fn name(&self) -> &str;

    /// Called before a tool is executed.
    /// Return a HookResult to modify the input, abort execution, or pass through.
    async fn before_tool(&self, ctx: &HookContext) -> Result<HookResult>;

    /// Called after a tool is executed.
    /// Can modify the output or log execution details.
    async fn after_tool(
        &self,
        ctx: &HookContext,
        output: &serde_json::Value,
    ) -> Result<serde_json::Value>;
}

/// A registry for managing hooks.
pub struct HookRegistry {
    hooks: Vec<Box<dyn Hook>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    pub fn register(&mut self, hook: Box<dyn Hook>) {
        tracing::info!("Registering hook: {}", hook.name());
        self.hooks.push(hook);
    }

    pub async fn run_before_hooks(&self, ctx: &HookContext) -> Result<HookResult> {
        for hook in &self.hooks {
            let result = hook.before_tool(ctx).await?;
            if result.abort {
                tracing::warn!(
                    "Hook {} aborted tool execution: {}",
                    hook.name(),
                    result.reason.as_deref().unwrap_or_default()
                );
                return Ok(result);
            }
        }
        Ok(HookResult::default())
    }

    pub async fn run_after_hooks(
        &self,
        ctx: &HookContext,
        mut output: serde_json::Value,
    ) -> Result<serde_json::Value> {
        for hook in &self.hooks {
            output = hook.after_tool(ctx, &output).await?;
        }
        Ok(output)
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}
