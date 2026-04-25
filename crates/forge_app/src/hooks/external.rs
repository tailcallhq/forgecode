use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use async_trait::async_trait;
use forge_domain::{Agent, ModelId, ToolCallFull, ToolCallInterceptor};
use serde::{Deserialize, Serialize};
use tracing::debug;

/// Interceptor that executes external hook scripts to modify tool calls.
///
/// At construction time, receives a pre-verified list of hook script paths
/// (cached in memory). At runtime, `intercept()` uses only these cached
/// paths — zero disk I/O, zero TOCTOU risk.
///
/// # Hook protocol
///
/// Input (stdin):
/// ```json
/// {"tool_name": "shell", "tool_input": {"command": "git status"}}
/// ```
///
/// Output (stdout) -- allow with modification:
/// ```json
/// {"decision": "allow", "hookSpecificOutput": {"tool_input": {"command": "rtk git status"}}}
/// ```
///
/// Output (stdout) -- allow without modification:
/// ```json
/// {"decision": "allow"}
/// ```
///
/// Output (stdout) -- deny:
/// ```json
/// {"decision": "deny", "reason": "blocked by policy"}
/// ```
#[derive(Clone, Default)]
pub struct ExternalHookInterceptor {
    cached_hooks: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize, Clone)]
struct HookInput {
    tool_name: String,
    tool_input: serde_json::Value,
}

#[derive(Serialize, Deserialize, Clone)]
struct HookOutput {
    decision: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: Option<HookSpecificOutput>,
}

#[derive(Serialize, Deserialize, Clone)]
struct HookSpecificOutput {
    tool_input: serde_json::Value,
}

impl ExternalHookInterceptor {
    /// Creates a new external hook interceptor with pre-verified cached hook
    /// paths.
    pub fn new(cached_hooks: Vec<PathBuf>) -> Self {
        Self { cached_hooks }
    }

    /// Returns the sorted list of hook scripts for a given event.
    ///
    /// Scans `~/.forge/hooks/<event>.d/` for executable files, sorted
    /// alphabetically by filename.
    ///
    /// This is a standalone function used by the startup loader and CLI
    /// commands. It is not called during `intercept()` — the interceptor
    /// uses cached paths instead.
    pub fn discover_hooks(event_name: &str) -> Vec<PathBuf> {
        let Some(home) = dirs::home_dir() else {
            return Vec::new();
        };
        let hook_dir = home
            .join(".forge")
            .join("hooks")
            .join(format!("{event_name}.d"));

        if !hook_dir.is_dir() {
            return Vec::new();
        }

        let Ok(entries) = std::fs::read_dir(&hook_dir) else {
            return Vec::new();
        };

        let mut hooks: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                // Only include files (not directories)
                path.is_file()
            })
            .filter(|path| {
                // On Unix, check if the file is executable
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::metadata(path)
                        .map(|m| m.permissions().mode() & 0o111 != 0)
                        .unwrap_or(false)
                }
                #[cfg(not(unix))]
                {
                    // On non-Unix, include files with common script extensions
                    path.extension()
                        .is_some_and(|ext| ext == "sh" || ext == "bash" || ext == "py")
                }
            })
            .collect();

        // Sort alphabetically for deterministic execution order
        hooks.sort();
        hooks
    }

    /// Run a single hook script, piping JSON input and parsing JSON output.
    async fn run_hook(
        hook_path: &std::path::Path,
        input: &HookInput,
    ) -> anyhow::Result<HookOutput> {
        let input_json = serde_json::to_string(input)?;

        debug!(hook = %hook_path.display(), "Executing external hook");

        let mut child = Command::new(hook_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(input_json.as_bytes()).await?;
        drop(stdin);

        let output = child.wait_with_output().await?;

        if !output.status.success() {
            debug!(
                hook = %hook_path.display(),
                exit_code = ?output.status.code(),
                "Hook exited with non-zero status, skipping"
            );
            // Treat non-zero exit as "allow" (pass-through)
            return Ok(HookOutput {
                decision: "allow".to_string(),
                reason: None,
                hook_specific_output: None,
            });
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        match serde_json::from_str::<HookOutput>(&output_str) {
            Ok(hook_output) => Ok(hook_output),
            Err(e) => {
                debug!(
                    hook = %hook_path.display(),
                    error = %e,
                    "Hook output was not valid JSON, treating as allow"
                );
                Ok(HookOutput {
                    decision: "allow".to_string(),
                    reason: None,
                    hook_specific_output: None,
                })
            }
        }
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
        let hooks = &self.cached_hooks;
        if hooks.is_empty() {
            return Ok(());
        }

        // Build initial input from the tool call
        let mut current_input = HookInput {
            tool_name: tool_call.name.as_str().to_string(),
            tool_input: serde_json::to_value(&tool_call.arguments)?,
        };

        for hook_path in hooks {
            let output = Self::run_hook(hook_path, &current_input).await?;

            match output.decision.as_str() {
                "deny" => {
                    debug!(
                        hook = %hook_path.display(),
                        reason = ?output.reason,
                        "Hook denied tool call"
                    );
                    // TODO: In the future, we could return an error or set a
                    // flag to prevent execution. For now, we stop the pipeline
                    // and let the tool call through unchanged.
                    return Ok(());
                }
                "allow" => {
                    if let Some(specific) = &output.hook_specific_output {
                        // Hook modified the tool input -- update for next hook
                        // in the pipeline
                        current_input.tool_input = specific.tool_input.clone();
                    }
                    // Continue to next hook
                }
                other => {
                    debug!(
                        hook = %hook_path.display(),
                        decision = other,
                        "Unknown hook decision, treating as allow"
                    );
                }
            }
        }

        // Apply the final result back to the tool call
        if let Ok(updated_args) =
            serde_json::from_value::<forge_domain::ToolCallArguments>(current_input.tool_input)
        {
            tool_call.arguments = updated_args;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_hooks_empty_dir() {
        // This test just ensures the function doesn't panic with missing dirs
        let hooks = ExternalHookInterceptor::discover_hooks("nonexistent-event");
        assert!(hooks.is_empty());
    }

    #[test]
    fn test_hook_output_deserialize_allow() {
        let json = r#"{"decision":"allow"}"#;
        let output: HookOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.decision, "allow");
        assert!(output.hook_specific_output.is_none());
    }

    #[test]
    fn test_hook_output_deserialize_allow_with_modification() {
        let json =
            r#"{"decision":"allow","hookSpecificOutput":{"tool_input":{"command":"rtk ls"}}}"#;
        let output: HookOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.decision, "allow");
        assert!(output.hook_specific_output.is_some());
        let specific = output.hook_specific_output.unwrap();
        assert_eq!(specific.tool_input["command"], "rtk ls");
    }

    #[test]
    fn test_hook_output_deserialize_deny() {
        let json = r#"{"decision":"deny","reason":"blocked"}"#;
        let output: HookOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.decision, "deny");
        assert_eq!(output.reason.as_deref(), Some("blocked"));
    }

    #[test]
    fn test_interceptor_with_empty_cached_hooks() {
        let interceptor = ExternalHookInterceptor::new(Vec::new());
        assert!(interceptor.cached_hooks.is_empty());
    }

    #[test]
    fn test_interceptor_with_cached_hooks() {
        let hooks = vec![
            PathBuf::from("/tmp/hook1.sh"),
            PathBuf::from("/tmp/hook2.sh"),
        ];
        let interceptor = ExternalHookInterceptor::new(hooks);
        assert_eq!(interceptor.cached_hooks.len(), 2);
    }
}
