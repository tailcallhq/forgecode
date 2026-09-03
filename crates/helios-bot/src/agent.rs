//! Bridge to the HeliosLite agent.
//!
//! Two execution paths:
//!
//! 1. **SDK path** (`run_agent_via_sdk`) — constructs a `ForgeAPI` in-process
//!    and calls `api.chat()` directly. No subprocess, no JSON parsing,
//!    no dependency on the `forge` binary being on PATH.
//!
//! 2. **Binary path** (`run_agent_via_binary`) — spawns
//!    `forge --request "..." --output-format json` and captures its output.
//!    Used as a fallback when the SDK path is unavailable (e.g. missing
//!    provider config, or `forge_api` features not compiled in).

use anyhow::{Context, Result};
use futures::stream::StreamExt;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// Outcome of running the agent on a `@helios` mention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentResult {
    /// The agent's response text to post back as a comment.
    pub response: String,
    /// Whether the agent created a PR (vs just commenting).
    pub created_pr: bool,
    /// PR number if one was created.
    pub pr_number: Option<u64>,
}

/// Run the HeliosLite agent on a request.
///
/// Tries the in-process SDK path first; falls back to the binary path if
/// SDK initialization fails (missing config, missing provider key, etc.).
#[allow(dead_code)]
pub async fn run_agent(repo_dir: &Path, request: &str, llm_api_key: &str) -> Result<AgentResult> {
    // Try SDK path first — faster, no subprocess overhead.
    match run_agent_via_sdk(repo_dir, request).await {
        Ok(result) => {
            tracing::info!("agent handled via SDK path");
            Ok(result)
        }
        Err(sdk_err) => {
            tracing::warn!("SDK path failed ({sdk_err}), falling back to binary");
            run_agent_via_binary(repo_dir, request, llm_api_key).await
        }
    }
}

/// In-process agent execution via forge_api.
///
/// Loads `ForgeConfig` from the repo directory, initialises `ForgeAPI`,
/// creates a new conversation, and streams the agent's response.
///
/// This path avoids subprocess overhead and JSON parsing — the entire
/// agent loop runs in the same process as the bot.
#[allow(dead_code)]
async fn run_agent_via_sdk(repo_dir: &Path, request: &str) -> Result<AgentResult> {
    use forge_api::{API as _, ForgeAPI};
    use forge_config::ForgeConfig;
    use forge_domain::{ChatRequest, ConversationId};

    let cwd = repo_dir
        .to_path_buf()
        .into_os_string()
        .into_string()
        .map_err(|_| anyhow::anyhow!("repo path contains non-UTF-8 characters"))?;

    let config = ForgeConfig::read().context("failed to read ForgeConfig")?;
    let cwd_path = std::path::PathBuf::from(&cwd);

    let api = ForgeAPI::init(cwd_path, config);

    // Create a new conversation for this interaction.
    let conversation_id = ConversationId::generate();
    let event = forge_domain::Event::new(request);

    let chat_req = ChatRequest::new(event, conversation_id);
    let mut stream = api.chat(chat_req).await.context("chat dispatch failed")?;

    // Collect the full response from the stream.
    let mut response_text = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("stream chunk error")?;
        match &chunk {
            forge_domain::ChatResponse::TaskMessage { content } => match content {
                forge_domain::ChatResponseContent::Markdown { text, .. } => {
                    response_text.push_str(text);
                }
                forge_domain::ChatResponseContent::ToolOutput(text) => {
                    response_text.push_str(text);
                }
                _ => {}
            },
            forge_domain::ChatResponse::TaskComplete => break,
            _ => {}
        }
    }

    if response_text.is_empty() {
        anyhow::bail!("agent returned empty response");
    }

    Ok(AgentResult { response: response_text, created_pr: false, pr_number: None })
}

/// Binary-path agent execution — spawns the `forge` CLI.
///
/// This is the reliable fallback: spawns `forge --request "..."` and
/// captures its JSON output. Works without any SDK dependencies.
#[allow(dead_code)]
async fn run_agent_via_binary(
    repo_dir: &Path,
    request: &str,
    llm_api_key: &str,
) -> Result<AgentResult> {
    let child = Command::new("forge")
        .arg("--request")
        .arg(request)
        .arg("--output-format")
        .arg("json")
        .current_dir(repo_dir)
        .env("LLM_API_KEY", llm_api_key)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn forge binary")?;

    let output = child.wait_with_output().await?;
    if !output.status.success() {
        let stderr = bstr::ByteSlice::to_str_lossy(&output.stderr[..]).replace('\0', "");
        anyhow::bail!("forge exited {}: {}", output.status, stderr);
    }

    // Parse the JSON response.  We accept either the canonical schema
    // (`{"response": "...", "pr_number": ...}`) or just plain text on stdout.
    let stdout = bstr::ByteSlice::to_str_lossy(&output.stdout[..]).replace('\0', "");
    if let Ok(parsed) = serde_json::from_str::<ForgeJsonOutput>(&stdout) {
        Ok(AgentResult {
            response: parsed.response,
            created_pr: parsed.pr_number.is_some(),
            pr_number: parsed.pr_number,
        })
    } else {
        // Plain text fallback.
        Ok(AgentResult { response: stdout, created_pr: false, pr_number: None })
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize)]
struct ForgeJsonOutput {
    response: String,
    #[serde(default)]
    pr_number: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stub that bypasses both SDK and binary paths.
    /// Useful when the test environment doesn't have `forge` on PATH.
    pub async fn run_agent_stub(repo_dir: &Path, request: &str) -> Result<AgentResult> {
        let _ = repo_dir;
        Ok(AgentResult {
            response: format!("[stub] received request: {request}"),
            created_pr: false,
            pr_number: None,
        })
    }

    #[tokio::test]
    async fn stub_returns_input() {
        let r = run_agent_stub(Path::new("."), "hello world").await.unwrap();
        assert!(r.response.contains("hello world"));
        assert_eq!(r.pr_number, None);
    }
}
