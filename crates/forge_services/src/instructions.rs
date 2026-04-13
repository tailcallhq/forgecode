use std::path::PathBuf;
use std::sync::Arc;

use forge_app::{CommandInfra, CustomInstructionsService, EnvironmentInfra, FileReaderInfra};

/// This service looks for AGENTS.md files in three locations in order of
/// priority:
/// 1. Base path (environment.base_path)
/// 2. Git root directory (if available)
/// 3. Current working directory (environment.cwd)
#[derive(Clone)]
pub struct ForgeCustomInstructionsService<F> {
    infra: Arc<F>,
    cache: tokio::sync::OnceCell<Vec<String>>,
}

impl<F: EnvironmentInfra + FileReaderInfra + CommandInfra> ForgeCustomInstructionsService<F> {
    pub fn new(infra: Arc<F>) -> Self {
        Self { infra, cache: Default::default() }
    }

    async fn discover_agents_files(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        let environment = self.infra.get_environment();

        // Base custom instructions
        let base_agent_md = environment.global_agentsmd_path();
        if !paths.contains(&base_agent_md) {
            paths.push(base_agent_md);
        }

        // Repo custom instructions
        if let Some(git_root_path) = self.get_git_root().await {
            let git_agent_md = git_root_path.join("AGENTS.md");
            if !paths.contains(&git_agent_md) {
                paths.push(git_agent_md);
            }
        }

        // Working dir custom instructions
        let cwd_agent_md = environment.local_agentsmd_path();
        if !paths.contains(&cwd_agent_md) {
            paths.push(cwd_agent_md);
        }

        paths
    }

    async fn get_git_root(&self) -> Option<PathBuf> {
        let output = self
            .infra
            .execute_command(
                "git rev-parse --show-toplevel".to_owned(),
                self.infra.get_environment().cwd,
                true, // silent mode - don't print git output
                None, // no environment variables needed for git command
            )
            .await
            .ok()?;

        if output.success() {
            Some(PathBuf::from(output.stdout.trim()))
        } else {
            None
        }
    }

    async fn init(&self) -> Vec<String> {
        let paths = self.discover_agents_files().await;

        let mut custom_instructions = Vec::new();

        for path in paths {
            if let Ok(content) = self.infra.read_utf8(&path).await {
                custom_instructions.push(content);
            }
        }

        custom_instructions
    }
}

#[async_trait::async_trait]
impl<F: EnvironmentInfra + FileReaderInfra + CommandInfra> CustomInstructionsService
    for ForgeCustomInstructionsService<F>
{
    async fn get_custom_instructions(&self) -> Vec<String> {
        self.cache.get_or_init(|| self.init()).await.clone()
    }
}
