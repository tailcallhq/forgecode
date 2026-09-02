//! Built-in commands forge executes for an ACP client.
//!
//! The terminal registers roughly forty slash commands, but most of them are
//! terminal affordances (`/exit`, `/copy`, `/edit`) or belong to the client in
//! an ACP session (`/new`, `/model`, mode switches — the client owns threads,
//! model selection and modes). This table is the set forge itself can execute
//! for a client, and each entry calls the same service the terminal calls.
//!
//! A command absent from this table is deliberately not advertised, so the
//! client can offer only what works instead of surfacing a command that fails.
use std::fmt::Write;

use agent_client_protocol as acp;
use forge_config::ForgeConfig;
use forge_domain::SyncProgress;
use futures::StreamExt;

use super::adapter::{AcpAdapter, SessionState};
use crate::{
    ConversationService, EnvironmentInfra, ForgeApp, GitApp, Services, WorkspaceService,
};

/// Name, description, and whether the command reads the rest of the prompt as
/// free-form input.
const BUILTIN_COMMANDS: &[(&str, &str, bool)] = &[
    ("compact", "Compact the conversation context", false),
    ("commit", "Generate an AI commit message and commit changes", true),
    ("commit-preview", "Preview an AI-generated commit message", true),
    ("info", "Display session and environment information", false),
    ("tools", "List the tools available to this agent", false),
    ("usage", "Show token usage and cost for this conversation", false),
    (
        "workspace-info",
        "Show workspace information with sync details",
        false,
    ),
    (
        "workspace-status",
        "Show sync status of all workspace files",
        false,
    ),
    (
        "workspace-sync",
        "Sync current workspace for semantic search",
        false,
    ),
];

/// First sentence of a tool description, short enough for a list entry.
fn summarize(description: &str) -> String {
    let line = description.lines().next().unwrap_or_default().trim();
    let end = line.find(". ").map(|index| index + 1).unwrap_or(line.len());
    let summary = &line[..end];
    if summary.chars().count() > 120 {
        format!("{}…", summary.chars().take(119).collect::<String>().trim_end())
    } else {
        summary.to_string()
    }
}

/// The built-in commands, as ACP advertises them.
pub(super) fn builtin_commands() -> impl Iterator<Item = acp::AvailableCommand> {
    BUILTIN_COMMANDS
        .iter()
        .map(|(name, description, takes_input)| {
            let command = acp::AvailableCommand::new(*name, *description);
            if *takes_input {
                command.input(acp::AvailableCommandInput::Unstructured(
                    acp::UnstructuredCommandInput::new("arguments"),
                ))
            } else {
                command
            }
        })
}

impl<S: Services + EnvironmentInfra<Config = ForgeConfig>> AcpAdapter<S> {
    /// Runs `name` when it is a built-in, returning the text to show the user.
    /// `None` means the name belongs to a custom command or a plain prompt.
    pub(super) async fn run_builtin_command(
        &self,
        session: &SessionState,
        name: &str,
        arguments: &str,
    ) -> Option<anyhow::Result<String>> {
        Some(match name {
            "compact" => self.builtin_compact(session).await,
            "commit" => self.builtin_commit(false, arguments).await,
            "commit-preview" => self.builtin_commit(true, arguments).await,
            "info" => Ok(self.builtin_info(session)),
            "tools" => self.builtin_tools().await,
            "usage" => self.builtin_usage(session).await,
            "workspace-info" => self.builtin_workspace_info().await,
            "workspace-status" => self.builtin_workspace_status().await,
            "workspace-sync" => self.builtin_workspace_sync().await,
            _ => return None,
        })
    }

    async fn builtin_compact(&self, session: &SessionState) -> anyhow::Result<String> {
        let result = ForgeApp::new(self.services.clone())
            .compact_conversation(session.agent_id.clone(), &session.conversation_id)
            .await?;
        Ok(format!(
            "Compacted the conversation: {} → {} messages, {} → {} tokens.",
            result.original_messages,
            result.compacted_messages,
            result.original_tokens,
            result.compacted_tokens,
        ))
    }

    async fn builtin_commit(&self, preview: bool, arguments: &str) -> anyhow::Result<String> {
        let additional_context = Some(arguments.trim())
            .filter(|arguments| !arguments.is_empty())
            .map(ToString::to_string);
        let git = GitApp::new(self.services.clone());
        let proposed = git.commit_message(None, None, additional_context).await?;
        if preview {
            return Ok(format!("Proposed commit message:\n\n{}", proposed.message));
        }

        let use_forge_committer = self.services.get_config()?.use_forge_committer;
        let result = git
            .commit(proposed.message, proposed.has_staged_files, use_forge_committer)
            .await?;
        Ok(if result.git_output.trim().is_empty() {
            format!("Committed:\n\n{}", result.message)
        } else {
            result.git_output
        })
    }

    fn builtin_info(&self, session: &SessionState) -> String {
        let environment = self.services.get_environment();
        let model = session
            .model_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "agent default".to_string());
        format!(
            "Agent: {}\nModel: {}\nWorking directory: {}\nOS: {}\nForge home: {}",
            session.agent_id,
            model,
            environment.cwd.display(),
            environment.os,
            environment.base_path.display(),
        )
    }

    async fn builtin_tools(&self) -> anyhow::Result<String> {
        let tools = ForgeApp::new(self.services.clone()).list_tools().await?;
        let mut out = format!(
            "{} system tools, {} agent tools.\n",
            tools.system.len(),
            tools.agents.len()
        );
        for tool in tools.system.iter().chain(tools.agents.iter()) {
            writeln!(out, "- `{}` — {}", tool.name, summarize(&tool.description))?;
        }
        Ok(out)
    }

    async fn builtin_usage(&self, session: &SessionState) -> anyhow::Result<String> {
        let conversation = self
            .services
            .find_conversation(&session.conversation_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Conversation not found"))?;
        let Some(usage) = conversation.accumulated_usage() else {
            return Ok("No usage recorded for this conversation yet.".to_string());
        };
        let mut out = format!(
            "Tokens: {} prompt + {} completion = {} total ({} cached).",
            usage.prompt_tokens, usage.completion_tokens, usage.total_tokens, usage.cached_tokens,
        );
        if let Some(cost) = usage.cost {
            write!(out, " Cost: ${cost:.4}.")?;
        }
        Ok(out)
    }

    async fn builtin_workspace_info(&self) -> anyhow::Result<String> {
        let cwd = self.services.get_environment().cwd.clone();
        let Some(info) = self.services.get_workspace_info(cwd).await? else {
            return Ok(
                "This directory has no workspace yet. Run /workspace-sync to index it.".to_string(),
            );
        };
        let mut out = format!(
            "Workspace {} at {}",
            info.workspace_id, info.working_dir
        );
        if let Some(nodes) = info.node_count {
            write!(out, "\nIndexed nodes: {nodes}")?;
        }
        if let Some(last_updated) = info.last_updated {
            write!(out, "\nLast updated: {last_updated}")?;
        }
        Ok(out)
    }

    async fn builtin_workspace_status(&self) -> anyhow::Result<String> {
        let cwd = self.services.get_environment().cwd.clone();
        let files = self.services.get_workspace_status(cwd).await?;
        if files.is_empty() {
            return Ok("No workspace files are tracked for semantic search.".to_string());
        }
        let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for file in &files {
            *counts.entry(format!("{:?}", file.status)).or_default() += 1;
        }
        let summary = counts
            .into_iter()
            .map(|(status, count)| format!("{count} {status}"))
            .collect::<Vec<_>>()
            .join(", ");
        Ok(format!("{} workspace files: {}.", files.len(), summary))
    }

    async fn builtin_workspace_sync(&self) -> anyhow::Result<String> {
        let cwd = self.services.get_environment().cwd.clone();
        let mut progress = self.services.sync_workspace(cwd).await?;
        let mut last = None;
        while let Some(update) = progress.next().await {
            last = Some(update?);
        }
        Ok(match last {
            Some(SyncProgress::Completed { total_files, uploaded_files, failed_files }) => format!(
                "Workspace synced: {uploaded_files} of {total_files} files uploaded, {failed_files} failed."
            ),
            _ => "Workspace sync ended without a completion report.".to_string(),
        })
    }
}
