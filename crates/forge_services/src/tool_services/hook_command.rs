use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use forge_app::{CommandInfra, HookCommandService};
use forge_domain::CommandOutput;

/// Thin wrapper around [`CommandInfra::execute_command_with_input`] that
/// satisfies the [`HookCommandService`] contract.
///
/// By delegating to the underlying infra this service avoids duplicating
/// process-spawning and stdin-piping logic; those concerns live entirely inside
/// the `CommandInfra` implementation.
#[derive(Clone)]
pub struct ForgeHookCommandService<I>(Arc<I>);

impl<I> ForgeHookCommandService<I> {
    /// Creates a new `ForgeHookCommandService` backed by the given infra.
    pub fn new(infra: Arc<I>) -> Self {
        Self(infra)
    }
}

#[async_trait::async_trait]
impl<I: CommandInfra> HookCommandService for ForgeHookCommandService<I> {
    async fn execute_command_with_input(
        &self,
        command: String,
        working_dir: PathBuf,
        stdin_input: String,
        env_vars: HashMap<String, String>,
    ) -> anyhow::Result<CommandOutput> {
        self.0
            .execute_command_with_input(command, working_dir, stdin_input, env_vars)
            .await
    }
}
