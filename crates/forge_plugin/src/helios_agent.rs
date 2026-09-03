use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::process::Command;

use crate::hook::{Hook, HookContext, HookResult};

/// Configuration for the Helios agent hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeliosAgentConfig {
    /// Path to the helios CLI binary (default: "helios").
    pub helios_bin: String,
    /// Session ID prefix for forge-initiated sessions.
    pub session_prefix: String,
    /// List of tool names to intercept and route to helios.
    /// If empty, all tools are intercepted.
    pub intercept_tools: Vec<String>,
    /// Timeout for helios subprocess calls in seconds.
    pub timeout_secs: u64,
}

impl Default for HeliosAgentConfig {
    fn default() -> Self {
        Self {
            helios_bin: "helios".into(),
            session_prefix: "forge".into(),
            intercept_tools: Vec::new(),
            timeout_secs: 30,
        }
    }
}

/// A hook that routes selected tool calls to the helios CLI agent via subprocess.
///
/// On `before_tool`, if the tool name matches the intercept list, the hook spawns
/// `helios run "<prompt>" --session-id forge-{session_id}` and returns the output
/// as the tool result, short-circuiting normal execution.
///
/// On `after_tool`, any helios-produced context is merged into the output when the
/// tool was not intercepted (pass-through for non-helios tools).
pub struct HeliosAgentHook {
    config: HeliosAgentConfig,
}

impl HeliosAgentHook {
    pub fn new(config: HeliosAgentConfig) -> Self {
        Self { config }
    }

    /// Returns `true` if the given tool should be routed to helios.
    fn should_intercept(&self, tool_name: &str) -> bool {
        self.config.intercept_tools.is_empty()
            || self.config.intercept_tools.iter().any(|t| t == tool_name)
    }
}

#[async_trait]
impl Hook for HeliosAgentHook {
    fn name(&self) -> &str {
        "helios-agent"
    }

    async fn before_tool(&self, ctx: &HookContext) -> Result<HookResult> {
        if !self.should_intercept(&ctx.tool_name) {
            return Ok(HookResult::default());
        }

        // Build the prompt from the tool name and input payload.
        let input_json = serde_json::to_string_pretty(&ctx.input).unwrap_or_else(|_| "{}".into());
        let prompt = format!("Tool call: {}\nInput: {}", ctx.tool_name, input_json);

        // Derive a per-session ID so helios can maintain conversation state.
        let session_id = ctx
            .metadata
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        let full_session_id = format!("{}-{}", self.config.session_prefix, session_id);

        tracing::info!(
            "Routing tool '{}' to helios agent (session: {})",
            ctx.tool_name,
            full_session_id
        );

        let raw_output = run_helios(
            &self.config.helios_bin,
            &prompt,
            &full_session_id,
            self.config.timeout_secs,
        )
        .await?;

        // Try to parse as JSON; fall back to a string value.
        let parsed: serde_json::Value =
            serde_json::from_str(&raw_output).unwrap_or(serde_json::Value::String(raw_output));

        Ok(HookResult {
            modified: true,
            output: Some(parsed),
            abort: false,
            reason: None,
        })
    }

    async fn after_tool(
        &self,
        _ctx: &HookContext,
        output: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        // For non-intercepted tools, pass through the output unchanged.
        // Helios context is already injected in `before_tool` for intercepted calls.
        Ok(output.clone())
    }
}

/// Spawn `helios run <prompt> --session-id <session_id>` and return its stdout.
async fn run_helios(
    bin: &str,
    prompt: &str,
    session_id: &str,
    timeout_secs: u64,
) -> Result<String> {
    let mut cmd = Command::new(bin);
    cmd.arg("run")
        .arg(prompt)
        .arg("--session-id")
        .arg(session_id)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let result = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), cmd.output())
        .await
        .with_context(|| format!("Helios subprocess timed out after {timeout_secs}s"))?
        .with_context(|| "Failed to spawn helios subprocess")?;

    if !result.status.success() {
        let stderr = bstr::ByteSlice::to_str_lossy(&result.stderr[..]);
        anyhow::bail!("Helios exited with {}: {}", result.status, stderr);
    }

    String::from_utf8(result.stdout).with_context(|| "Helios output was not valid UTF-8")
}
