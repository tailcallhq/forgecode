use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use forge_domain::*;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::services::{
    AgentRegistry, AppConfigService, ProviderAuthService, ProviderService, ShellService,
    TemplateService,
};
use crate::{AgentProviderResolver, EnvironmentInfra, Services};

/// Errors specific to GitApp operations
#[derive(thiserror::Error, Debug)]
pub enum GitAppError {
    #[error("nothing to commit, working tree clean")]
    NoChangesToCommit,
}

/// GitApp handles git-related operations like commit message generation.
pub struct GitApp<S> {
    services: Arc<S>,
}

/// Result of a commit operation
#[derive(Debug, Clone)]
pub struct CommitResult {
    /// The generated commit message
    pub message: String,
    /// Whether the commit was actually executed (false for preview mode)
    pub committed: bool,
    /// Whether there are staged files (used internally)
    pub has_staged_files: bool,
    /// Output from git commit command (stdout + stderr)
    pub git_output: String,
}

/// Details about commit message generation
#[derive(Debug, Clone)]
struct CommitMessageDetails {
    /// The generated commit message
    message: String,
    /// Whether there are staged files
    has_staged_files: bool,
}

/// Structured response for commit message generation using JSON format
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(title = "commit_message")]
pub struct CommitMessageResponse {
    /// The commit message in conventional commit format
    pub commit_message: String,
}

/// Context for generating a commit message from a diff
#[derive(Debug, Clone)]
struct DiffContext {
    diff_content: String,
    branch_name: String,
    recent_commits: String,
    has_staged_files: bool,
    additional_context: Option<String>,
}

impl<S> GitApp<S> {
    /// Creates a new GitApp instance with the provided services.
    pub fn new(services: Arc<S>) -> Self {
        Self { services }
    }

    /// Truncates diff content if it exceeds the maximum size
    fn truncate_diff(
        &self,
        diff_content: String,
        max_diff_size: Option<usize>,
        original_size: usize,
    ) -> (String, bool) {
        match max_diff_size {
            Some(max_size) if original_size > max_size => {
                // Safely truncate at a char boundary
                let truncated = diff_content
                    .char_indices()
                    .take_while(|(idx, _)| *idx < max_size)
                    .map(|(_, c)| c)
                    .collect::<String>();
                (truncated, true)
            }
            _ => (diff_content, false),
        }
    }
}

impl<S: Services + EnvironmentInfra<Config = forge_config::ForgeConfig>> GitApp<S> {
    /// Generates a commit message without committing
    ///
    /// # Arguments
    ///
    /// * `max_diff_size` - Maximum size of git diff in bytes. None for
    ///   unlimited.
    /// * `diff` - Optional diff content provided via pipe. If provided, this
    ///   diff is used instead of fetching from git.
    /// * `additional_context` - Optional additional text to help structure the
    ///   commit message
    ///
    /// # Errors
    ///
    /// Returns an error if git operations fail or AI generation fails
    pub async fn commit_message(
        &self,
        max_diff_size: Option<usize>,
        diff: Option<String>,
        additional_context: Option<String>,
    ) -> Result<CommitResult> {
        let CommitMessageDetails { message, has_staged_files } = self
            .generate_commit_message(max_diff_size, diff, additional_context)
            .await?;

        Ok(CommitResult {
            message,
            committed: false,
            has_staged_files,
            git_output: String::new(),
        })
    }

    /// Commits changes with the provided commit message.
    ///
    /// When `use_forge_committer` is true, sets ForgeCode as the Git committer
    /// via `GIT_COMMITTER_NAME` and `GIT_COMMITTER_EMAIL` environment
    /// variables while preserving the user as the author.
    ///
    /// # Arguments
    ///
    /// * `message` - The commit message to use
    /// * `has_staged_files` - Whether there are staged files
    /// * `use_forge_committer` - Whether to override the Git committer with
    ///   ForgeCode identity
    ///
    /// # Errors
    ///
    /// Returns an error if git commit fails
    pub async fn commit(
        &self,
        message: String,
        has_staged_files: bool,
        use_forge_committer: bool,
    ) -> Result<CommitResult> {
        let cwd = self.services.get_environment().cwd;
        let flags = if has_staged_files { "" } else { " -a" };
        let commit_command = build_commit_command(&message, flags, use_forge_committer);

        let commit_result = self
            .services
            .execute(commit_command, cwd, false, true, None, None)
            .await
            .context("Failed to commit changes")?;

        if !commit_result.output.success() {
            anyhow::bail!("Git commit failed: {}", commit_result.output.stderr);
        }

        // Combine stdout and stderr for logging
        let git_output = if commit_result.output.stdout.is_empty() {
            commit_result.output.stderr.clone()
        } else if commit_result.output.stderr.is_empty() {
            commit_result.output.stdout.clone()
        } else {
            format!(
                "{}\n{}",
                commit_result.output.stdout, commit_result.output.stderr
            )
        };

        Ok(CommitResult { message, committed: true, has_staged_files, git_output })
    }

    /// Generates a commit message based on staged git changes and returns
    /// details about the commit context
    async fn generate_commit_message(
        &self,
        max_diff_size: Option<usize>,
        diff: Option<String>,
        additional_context: Option<String>,
    ) -> Result<CommitMessageDetails> {
        // Get current working directory
        let cwd = self.services.get_environment().cwd;

        // Fetch git context (always needed for commit message generation)
        let (recent_commits, branch_name) = self.fetch_git_context(&cwd).await?;

        // Get diff content and metadata
        let (diff_content, original_size, has_staged_files) = if let Some(piped_diff) = diff {
            // Use piped diff
            let size = piped_diff.len();
            (piped_diff, size, false) // Assume unstaged for piped diff
        } else {
            // Fetch diff from git
            self.fetch_git_diff(&cwd).await?
        };

        // Truncate diff if it exceeds max size
        let (truncated_diff, _) = self.truncate_diff(diff_content, max_diff_size, original_size);

        let ctx = DiffContext {
            diff_content: truncated_diff,
            branch_name,
            recent_commits,
            has_staged_files,
            additional_context,
        };

        let retry_config = self.services.get_config()?.retry.unwrap_or_default();
        crate::retry::retry_with_config(
            &retry_config,
            || self.generate_message_from_diff(ctx.clone()),
            None::<fn(&anyhow::Error, std::time::Duration)>,
        )
        .await
    }

    /// Fetches git context (branch name and recent commits)
    async fn fetch_git_context(&self, cwd: &Path) -> Result<(String, String)> {
        let max_commit_count = self.services.get_config()?.max_commit_count;
        let git_log_cmd =
            format!("git log --pretty=format:%s --abbrev-commit --max-count={max_commit_count}");
        let (recent_commits, branch_name) = tokio::join!(
            self.services
                .execute(git_log_cmd, cwd.to_path_buf(), false, true, None, None,),
            self.services.execute(
                "git rev-parse --abbrev-ref HEAD".into(),
                cwd.to_path_buf(),
                false,
                true,
                None,
                None,
            ),
        );

        let recent_commits = recent_commits.context("Failed to get recent commits")?;
        let branch_name = branch_name.context("Failed to get branch name")?;

        Ok((recent_commits.output.stdout, branch_name.output.stdout))
    }

    /// Fetches diff from git (staged or unstaged)
    async fn fetch_git_diff(&self, cwd: &Path) -> Result<(String, usize, bool)> {
        let (staged_diff, unstaged_diff) = tokio::join!(
            self.services.execute(
                "git diff --staged".into(),
                cwd.to_path_buf(),
                false,
                true,
                None,
                None,
            ),
            self.services.execute(
                "git diff".into(),
                cwd.to_path_buf(),
                false,
                true,
                None,
                None,
            )
        );

        let staged_diff = staged_diff.context("Failed to get staged changes")?;
        let unstaged_diff = unstaged_diff.context("Failed to get unstaged changes")?;

        // Use staged changes if available, otherwise fall back to unstaged changes
        let has_staged_files = !staged_diff.output.stdout.trim().is_empty();
        let diff_output = if has_staged_files {
            staged_diff
        } else if !unstaged_diff.output.stdout.trim().is_empty() {
            unstaged_diff
        } else {
            return Err(GitAppError::NoChangesToCommit.into());
        };

        let size = diff_output.output.stdout.len();
        Ok((diff_output.output.stdout, size, has_staged_files))
    }

    /// Resolves the provider and model from the active agent's configuration.
    async fn resolve_agent_provider_and_model(
        &self,
        resolver: &AgentProviderResolver<S>,
        agent_id: Option<AgentId>,
    ) -> Result<(Provider<url::Url>, ModelId)> {
        let (provider_template, model) = tokio::try_join!(
            resolver.get_provider(agent_id.clone()),
            resolver.get_model(agent_id)
        )?;
        let provider = self
            .services
            .refresh_provider_credential(provider_template)
            .await?;
        Ok((provider, model))
    }

    /// Generates a commit message from the provided diff and git context
    async fn generate_message_from_diff(&self, ctx: DiffContext) -> Result<CommitMessageDetails> {
        let (agent_id, commit_config) = tokio::try_join!(
            self.services.get_active_agent_id(),
            self.services.get_commit_config()
        )?;
        let agent_provider_resolver = AgentProviderResolver::new(self.services.clone());

        // Resolve provider and model: commit config takes priority over agent defaults.
        // If the configured provider is unavailable (e.g. logged out), fall back to the
        // agent's provider/model with a warning.
        let (provider, model) = match commit_config {
            Some(mc) => match self.services.get_provider(mc.provider).await {
                Ok(provider) => match self.services.refresh_provider_credential(provider).await {
                    Ok(provider) => (provider, mc.model),
                    Err(err) => {
                        tracing::warn!(
                            error = %err,
                            "Failed to refresh credentials for configured commit provider. Falling back to the active provider."
                        );
                        self.resolve_agent_provider_and_model(&agent_provider_resolver, agent_id)
                            .await?
                    }
                },
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "Configured commit provider unavailable. Falling back to the active provider."
                    );
                    self.resolve_agent_provider_and_model(&agent_provider_resolver, agent_id)
                        .await?
                }
            },
            None => {
                self.resolve_agent_provider_and_model(&agent_provider_resolver, agent_id)
                    .await?
            }
        };

        let rendered_prompt = self
            .services
            .render_template(Template::new("{{> forge-commit-message-prompt.md }}"), &())
            .await?;

        // Build user message using structured JSON format
        let user_data = serde_json::json!({
            "branch_name": ctx.branch_name,
            "recent_commit_messages": ctx.recent_commits,
            "git_diff": ctx.diff_content,
            "additional_context": ctx.additional_context
        });

        // Generate JSON schema from CommitMessageResponse using schemars
        let schema = schemars::schema_for!(CommitMessageResponse);

        let context = forge_domain::Context::default()
            .add_message(ContextMessage::system(rendered_prompt))
            .add_message(ContextMessage::user(
                serde_json::to_string(&user_data)?,
                Some(model.clone()),
            ))
            .response_format(ResponseFormat::JsonSchema(Box::new(schema)));

        // Send message to LLM
        let stream = self.services.chat(&model, context, provider).await?;
        let message = stream.into_full(false).await?;

        // Parse the response - try JSON first (structured output), fallback to plain
        // text
        let commit_message = match serde_json::from_str::<CommitMessageResponse>(&message.content) {
            Ok(response) => response.commit_message,
            Err(_) => {
                // Fallback: Some providers don't support structured output, treat as plain text
                message.content.trim().to_string()
            }
        };

        if commit_message.is_empty() {
            return Err(Error::Retryable(anyhow::anyhow!("Empty commit message generated")).into());
        }

        Ok(CommitMessageDetails {
            message: commit_message,
            has_staged_files: ctx.has_staged_files,
        })
    }
}

/// Builds the `git commit` shell command string.
///
/// When `use_forge_committer` is true, prefixes the command with
/// `GIT_COMMITTER_NAME` and `GIT_COMMITTER_EMAIL` environment variables
/// to set ForgeCode as the committer.
fn build_commit_command(message: &str, flags: &str, use_forge_committer: bool) -> String {
    // Escape single quotes in the message by replacing ' with '\''
    let escaped_message = message.replace('\'', r"'\''");
    if use_forge_committer {
        format!(
            "GIT_COMMITTER_NAME='ForgeCode' GIT_COMMITTER_EMAIL='noreply@forgecode.dev' git commit {flags} -m '{escaped_message}'"
        )
    } else {
        format!("git commit {flags} -m '{escaped_message}'")
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_build_commit_command_with_forge_committer_staged() {
        let actual = build_commit_command("feat: add feature", "", true);
        let expected = "GIT_COMMITTER_NAME='ForgeCode' GIT_COMMITTER_EMAIL='noreply@forgecode.dev' git commit  -m 'feat: add feature'";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_build_commit_command_with_forge_committer_unstaged() {
        let actual = build_commit_command("fix: bug", " -a", true);
        let expected = "GIT_COMMITTER_NAME='ForgeCode' GIT_COMMITTER_EMAIL='noreply@forgecode.dev' git commit  -a -m 'fix: bug'";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_build_commit_command_without_forge_committer_staged() {
        let actual = build_commit_command("chore: update", "", false);
        let expected = "git commit  -m 'chore: update'";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_build_commit_command_without_forge_committer_unstaged() {
        let actual = build_commit_command("docs: readme", " -a", false);
        let expected = "git commit  -a -m 'docs: readme'";
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_build_commit_command_escapes_single_quotes() {
        let actual = build_commit_command("feat: it's done", "", true);
        let expected = "GIT_COMMITTER_NAME='ForgeCode' GIT_COMMITTER_EMAIL='noreply@forgecode.dev' git commit  -m 'feat: it'\\''s done'";
        assert_eq!(actual, expected);
    }
}
