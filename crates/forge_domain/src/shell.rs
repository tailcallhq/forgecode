/// Output from a command execution
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    /// Wall-clock time in seconds the command took to execute
    pub wall_time_secs: Option<f64>,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.exit_code.is_none_or(|code| code >= 0)
    }
}
