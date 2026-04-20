use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use async_trait::async_trait;
use forge_domain::{Agent, ModelId, ToolCallFull, ToolCallInterceptor};
use serde::{Deserialize, Serialize};

/// Interceptor that executes external scripts to modify tool calls.
///
/// Looks for scripts in `~/.forge/hooks/` and executes them if they exist.
/// Currently supports the `rtk-toolcall-start.sh` script for command
/// rewriting.
#[derive(Clone, Default)]
pub struct ExternalHookInterceptor;

#[derive(Serialize, Deserialize)]
struct ExternalHookInput {
    tool_name: String,
    tool_input: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
struct ExternalHookOutput {
    decision: String,
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: Option<HookSpecificOutput>,
}

#[derive(Serialize, Deserialize)]
struct HookSpecificOutput {
    tool_input: serde_json::Value,
}

impl ExternalHookInterceptor {
    /// Creates a new external hook interceptor
    pub fn new() -> Self {
        Self
    }

    fn get_hook_path(event_name: &str) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        let hook_path = home
            .join(".forge")
            .join("hooks")
            .join(format!("rtk-{}.sh", event_name));
        hook_path.exists().then_some(hook_path)
    }
}

#[async_trait]
impl ToolCallInterceptor for ExternalHookInterceptor {
    async fn intercept(
        &self,
        tool_call: &mut ToolCallFull,
        _agent: &Agent,
        _model_id: &ModelId,
    ) -> anyhow::Result<()> {
        let Some(hook_path) = Self::get_hook_path("toolcall-start") else {
            return Ok(());
        };

        let input = ExternalHookInput {
            tool_name: tool_call.name.as_str().to_string(),
            tool_input: serde_json::to_value(&tool_call.arguments)?,
        };

        let mut child = Command::new(hook_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let mut stdin = child.stdin.take().unwrap();
        let input_json = serde_json::to_string(&input)?;
        stdin.write_all(input_json.as_bytes()).await?;
        drop(stdin);

        let output = child.wait_with_output().await?;
        if output.status.success() {
            let output_str = String::from_utf8_lossy(&output.stdout);
            if let Ok(external_output) = serde_json::from_str::<ExternalHookOutput>(&output_str) {
                if external_output.decision == "allow" {
                    if let Some(specific) = external_output.hook_specific_output {
                        let updated_args = serde_json::from_value(specific.tool_input)?;
                        tool_call.arguments = updated_args;
                    }
                }
            }
        }

        Ok(())
    }
}
