use derive_setters::Setters;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const DEFAULT_SHELL_BEHAVIOR_QUIET: bool = true;
pub const DEFAULT_SHELL_BEHAVIOR_SYNC: bool = false;

/// Controls presentation and background behavior for shell-triggered prompts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, fake::Dummy, Setters)]
#[serde(rename_all = "snake_case")]
#[setters(strip_option)]
pub struct ShellBehaviorConfig {
    /// Suppresses transient UI output such as reasoning, initialization titles,
    /// and finished markers for shell-triggered prompt executions.
    #[serde(default)]
    pub quiet: bool,

    /// Enables automatic background workspace sync after a shell-triggered
    /// prompt completes.
    #[serde(default = "default_shell_behavior_sync")]
    pub sync: bool,
}

impl Default for ShellBehaviorConfig {
    fn default() -> Self {
        Self {
            quiet: DEFAULT_SHELL_BEHAVIOR_QUIET,
            sync: DEFAULT_SHELL_BEHAVIOR_SYNC,
        }
    }
}

fn default_shell_behavior_sync() -> bool {
    DEFAULT_SHELL_BEHAVIOR_SYNC
}
