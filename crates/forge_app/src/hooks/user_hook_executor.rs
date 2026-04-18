use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use forge_domain::{CommandOutput, HookExecutionResult};
use tracing::debug;

use crate::services::HookCommandService;

/// Executes user hook commands by delegating to a [`HookCommandService`].
///
/// Holds the service by value; the service itself is responsible for any
/// internal reference counting (`Arc`). Keeps hook-specific timeout resolution
/// in one place.
#[derive(Clone)]
pub struct UserHookExecutor<S>(S);

impl<S> UserHookExecutor<S> {
    /// Creates a new `UserHookExecutor` backed by the given service.
    pub fn new(service: S) -> Self {
        Self(service)
    }
}

impl<S: HookCommandService> UserHookExecutor<S> {
    /// Executes a shell command, piping `input_json` to stdin and capturing
    /// stdout/stderr.
    ///
    /// Applies `timeout_duration` by racing the service call against the
    /// deadline. On timeout, returns a `HookExecutionResult` with
    /// `exit_code: None` and a descriptive message in `stderr`.
    ///
    /// # Arguments
    /// * `command` - The shell command string to execute.
    /// * `input_json` - JSON string to pipe to the command's stdin.
    /// * `timeout_duration` - Maximum time to wait for the command.
    /// * `cwd` - Working directory for the command.
    /// * `env_vars` - Additional environment variables to set.
    ///
    /// # Errors
    /// Returns an error if the process cannot be spawned.
    pub async fn execute(
        &self,
        command: &str,
        input_json: &str,
        timeout_duration: Duration,
        cwd: &Path,
        env_vars: &HashMap<String, String>,
    ) -> anyhow::Result<HookExecutionResult> {
        debug!(
            command = command,
            cwd = %cwd.display(),
            timeout_ms = timeout_duration.as_millis() as u64,
            "Executing user hook command"
        );

        let result = tokio::time::timeout(
            timeout_duration,
            self.0.execute_command_with_input(
                command.to_string(),
                cwd.to_path_buf(),
                input_json.to_string(),
                env_vars.clone(),
            ),
        )
        .await;

        let output = match result {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                tracing::warn!(
                    command = command,
                    timeout_ms = timeout_duration.as_millis() as u64,
                    "Hook command timed out"
                );
                CommandOutput {
                    command: command.to_string(),
                    exit_code: None,
                    stdout: String::new(),
                    stderr: format!(
                        "Hook command timed out after {}ms",
                        timeout_duration.as_millis()
                    ),
                }
            }
        };

        debug!(
            command = command,
            exit_code = ?output.exit_code,
            stdout_len = output.stdout.len(),
            stderr_len = output.stderr.len(),
            "Hook command completed"
        );

        Ok(HookExecutionResult {
            exit_code: output.exit_code,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::Duration;

    use forge_domain::CommandOutput;
    use pretty_assertions::assert_eq;

    use super::*;

    /// A minimal service stub that records calls and returns a fixed result.
    #[derive(Clone)]
    struct StubInfra {
        result: CommandOutput,
    }

    impl StubInfra {
        fn success(stdout: &str) -> Self {
            Self {
                result: CommandOutput {
                    command: String::new(),
                    exit_code: Some(0),
                    stdout: stdout.to_string(),
                    stderr: String::new(),
                },
            }
        }

        fn exit(code: i32, stderr: &str) -> Self {
            Self {
                result: CommandOutput {
                    command: String::new(),
                    exit_code: Some(code),
                    stdout: String::new(),
                    stderr: stderr.to_string(),
                },
            }
        }

        fn timeout() -> Self {
            Self {
                result: CommandOutput {
                    command: String::new(),
                    exit_code: None,
                    stdout: String::new(),
                    stderr: "Hook command timed out after 100ms".to_string(),
                },
            }
        }
    }

    #[async_trait::async_trait]
    impl HookCommandService for StubInfra {
        async fn execute_command_with_input(
            &self,
            command: String,
            _working_dir: PathBuf,
            _stdin_input: String,
            _env_vars: HashMap<String, String>,
        ) -> anyhow::Result<CommandOutput> {
            let mut out = self.result.clone();
            out.command = command;
            Ok(out)
        }
    }

    #[tokio::test]
    async fn test_execute_success() {
        let fixture = UserHookExecutor::new(StubInfra::success("hello"));
        let actual = fixture
            .execute(
                "echo hello",
                "{}",
                Duration::from_secs(0),
                &std::env::current_dir().unwrap(),
                &HashMap::new(),
            )
            .await
            .unwrap();

        assert_eq!(actual.exit_code, Some(0));
        assert_eq!(actual.stdout, "hello");
        assert!(actual.is_success());
    }

    #[tokio::test]
    async fn test_execute_exit_code_2() {
        let fixture = UserHookExecutor::new(StubInfra::exit(2, "blocked"));
        let actual = fixture
            .execute(
                "exit 2",
                "{}",
                Duration::from_secs(0),
                &std::env::current_dir().unwrap(),
                &HashMap::new(),
            )
            .await
            .unwrap();

        assert_eq!(actual.exit_code, Some(2));
        assert!(actual.is_blocking_exit());
        assert!(actual.stderr.contains("blocked"));
    }

    #[tokio::test]
    async fn test_execute_non_blocking_error() {
        let fixture = UserHookExecutor::new(StubInfra::exit(1, ""));
        let actual = fixture
            .execute(
                "exit 1",
                "{}",
                Duration::from_secs(0),
                &std::env::current_dir().unwrap(),
                &HashMap::new(),
            )
            .await
            .unwrap();

        assert_eq!(actual.exit_code, Some(1));
        assert!(actual.is_non_blocking_error());
    }

    #[tokio::test]
    async fn test_execute_timeout() {
        let fixture = UserHookExecutor::new(StubInfra::timeout());
        let actual = fixture
            .execute(
                "sleep 10",
                "{}",
                Duration::from_millis(100),
                &std::env::current_dir().unwrap(),
                &HashMap::new(),
            )
            .await
            .unwrap();

        assert!(actual.exit_code.is_none());
        assert!(actual.stderr.contains("timed out"));
    }
}
