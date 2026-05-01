//! Loop executor - trait and basic implementation for executing prompts

use crate::{LoopError, LoopId, Result};

/// Trait for executing loop prompts
///
/// Implement this trait to integrate with Forge's conversation system.
#[async_trait::async_trait]
pub trait LoopExecutor: Send + Sync {
    /// Execute a prompt in the given conversation
    async fn execute(&self, loop_id: &LoopId, conversation_id: &str, prompt: &str) -> Result<()>;
}

/// Simple executor that spawns forge with the prompt
pub struct ForgeLoopExecutor {
    forge_bin: String,
}

impl ForgeLoopExecutor {
    /// Create a new Forge loop executor
    pub fn new(forge_bin: impl Into<String>) -> Self {
        Self {
            forge_bin: forge_bin.into(),
        }
    }
}

#[async_trait::async_trait]
impl LoopExecutor for ForgeLoopExecutor {
    async fn execute(&self, loop_id: &LoopId, conversation_id: &str, prompt: &str) -> Result<()> {
        tracing::info!(
            loop_id = %loop_id,
            conversation = conversation_id,
            "Executing loop prompt"
        );

        let output = tokio::process::Command::new(&self.forge_bin)
            .args(["-p", prompt])
            .arg("--conversation-id")
            .arg(conversation_id)
            .output()
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!("Loop execution failed: {}", stderr);
            return Err(LoopError::ExecutionFailed(stderr.to_string()));
        }

        tracing::info!(loop_id = %loop_id, "Loop execution completed");
        Ok(())
    }
}

/// Mock executor for testing
#[cfg(test)]
pub struct MockExecutor {
    pub calls: std::sync::Arc<std::sync::Mutex<Vec<(LoopId, String, String)>>>,
}

#[cfg(test)]
impl MockExecutor {
    pub fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl LoopExecutor for MockExecutor {
    async fn execute(&self, loop_id: &LoopId, conversation_id: &str, prompt: &str) -> Result<()> {
        let mut calls = self.calls.lock().unwrap();
        calls.push((loop_id.clone(), conversation_id.to_string(), prompt.to_string()));
        Ok(())
    }
}
