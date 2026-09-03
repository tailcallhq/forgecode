//! Dual-harness shared fixture adapter for forgecode (FR-DH-001).
//!
//! Loads `shared-3task.v1.json` and executes the `forgecode` adapter specs
//! without model serving. Parity with helios-cli `harness_runner::dual_harness`
//! and Planify2 `scripts/dual-harness/run_shared_3task.cjs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

/// Fixture root document (`pheno.dual_harness.fixture.v1`).
#[derive(Debug, Clone, Deserialize)]
pub struct DualHarnessFixture {
    /// Schema id.
    pub schema_version: String,
    /// Stable fixture id.
    pub fixture_id: String,
    /// Tasks to run.
    pub tasks: Vec<FixtureTask>,
}

/// One fixture task.
#[derive(Debug, Clone, Deserialize)]
pub struct FixtureTask {
    /// Task id (e.g. `dh-t1-echo-ok`).
    pub task_id: String,
    /// Human title.
    pub title: String,
    /// Kind (`process_smoke`).
    pub kind: String,
    /// Acceptance predicates.
    pub acceptance: Acceptance,
    /// Per-harness adapter specs.
    pub adapters: HashMap<String, AdapterSpec>,
}

/// Acceptance predicates for a task.
#[derive(Debug, Clone, Deserialize)]
pub struct Acceptance {
    /// Required exit code.
    pub exit_code: Option<i32>,
    /// Substring that must appear in stdout.
    pub stdout_contains: Option<String>,
    /// Env var whose value must prefix stdout (realpath).
    pub stdout_path_prefix_env: Option<String>,
    /// When true, the run must fail.
    pub must_error: Option<bool>,
    /// Error class (`timeout`).
    pub error_class: Option<String>,
    /// Timeout seconds (also on adapter).
    pub timeout_secs: Option<u64>,
}

/// Process adapter spec.
#[derive(Debug, Clone, Deserialize)]
pub struct AdapterSpec {
    /// Executable.
    pub cmd: String,
    /// Args.
    #[serde(default)]
    pub args: Vec<String>,
    /// Env var naming the working directory.
    pub working_dir_env: Option<String>,
    /// Kill after N seconds.
    pub timeout_secs: Option<u64>,
}

/// Outcome of one fixture task.
#[derive(Debug, Clone)]
pub struct TaskOutcome {
    /// Task id.
    pub task_id: String,
    /// Pass/fail.
    pub passed: bool,
    /// Short detail.
    pub detail: String,
}

/// Fixture / run errors (fail loud).
#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    /// IO.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// JSON.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// Unsupported schema.
    #[error("fixture schema unsupported: {0}")]
    Schema(String),
    /// Missing forgecode adapter.
    #[error("task {0} missing forgecode adapter")]
    MissingAdapter(String),
    /// Workdir env unset.
    #[error("DUAL_HARNESS_WORKDIR unset (required for task {0})")]
    WorkdirUnset(String),
}

/// Load fixture JSON from disk.
pub fn load_fixture(path: &Path) -> Result<DualHarnessFixture, FixtureError> {
    let raw = std::fs::read_to_string(path)?;
    let fixture: DualHarnessFixture = serde_json::from_str(&raw)?;
    if fixture.schema_version != "pheno.dual_harness.fixture.v1" {
        return Err(FixtureError::Schema(fixture.schema_version));
    }
    Ok(fixture)
}

/// Default path to the vendored shared-3task fixture.
pub fn default_shared_3task_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("shared-3task.v1.json")
}

struct ProcRun {
    timed_out: bool,
    status: Option<i32>,
    stdout: String,
    #[allow(dead_code)] // retained for future fail-loud diagnostics
    stderr: String,
    error: Option<String>,
}

/// Run all forgecode adapter tasks.
pub async fn run_forgecode_fixture(
    fixture: &DualHarnessFixture,
) -> Result<Vec<TaskOutcome>, FixtureError> {
    let mut out = Vec::with_capacity(fixture.tasks.len());
    for task in &fixture.tasks {
        out.push(run_one(task).await?);
    }
    Ok(out)
}

async fn run_one(task: &FixtureTask) -> Result<TaskOutcome, FixtureError> {
    let adapter = task
        .adapters
        .get("forgecode")
        .ok_or_else(|| FixtureError::MissingAdapter(task.task_id.clone()))?;

    if let Some(env_key) = &adapter.working_dir_env
        && std::env::var_os(env_key).is_none()
    {
        return Err(FixtureError::WorkdirUnset(task.task_id.clone()));
    }

    let run = spawn_adapter(adapter).await?;
    let passed = accept(task, &run);
    let detail = if passed {
        "ok".into()
    } else if run.timed_out {
        "timeout".into()
    } else if let Some(err) = &run.error {
        err.clone()
    } else {
        format!("status={:?} stdout={:?}", run.status, run.stdout.trim())
    };
    Ok(TaskOutcome { task_id: task.task_id.clone(), passed, detail })
}

async fn spawn_adapter(adapter: &AdapterSpec) -> Result<ProcRun, FixtureError> {
    let timeout_secs = adapter.timeout_secs.unwrap_or(30);
    // On Windows `pwd` resolves to the MSYS shim, which prints /tmp-style
    // remapped paths that fail the canonicalize comparison against the native
    // workdir. `cmd /c cd` prints the current directory as a native path,
    // keeping the fixture (and its workdir-isolation check) platform-neutral.
    let mut cmd = if cfg!(windows) && adapter.cmd == "pwd" {
        let mut c = Command::new("cmd");
        c.args(["/c", "cd"]);
        c
    } else {
        let mut c = Command::new(&adapter.cmd);
        c.args(&adapter.args);
        c
    };
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);
    if let Some(env_key) = &adapter.working_dir_env {
        let dir = std::env::var(env_key)
            .map_err(|_| FixtureError::WorkdirUnset(format!("env {env_key}")))?;
        cmd.current_dir(dir);
    }

    let mut child = cmd.spawn()?;
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();

    let wait = timeout(Duration::from_secs(timeout_secs), async {
        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(ref mut out) = stdout_pipe {
            out.read_to_string(&mut stdout).await.ok();
        }
        if let Some(ref mut err) = stderr_pipe {
            err.read_to_string(&mut stderr).await.ok();
        }
        let status = child.wait().await?;
        Ok::<_, std::io::Error>((status, stdout, stderr))
    })
    .await;

    match wait {
        Ok(Ok((status, stdout, stderr))) => Ok(ProcRun {
            timed_out: false,
            status: status.code(),
            stdout,
            stderr,
            error: None,
        }),
        Ok(Err(e)) => Ok(ProcRun {
            timed_out: false,
            status: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(e.to_string()),
        }),
        Err(_elapsed) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            Ok(ProcRun {
                timed_out: true,
                status: None,
                stdout: String::new(),
                stderr: String::new(),
                error: Some("timeout".into()),
            })
        }
    }
}

fn accept(task: &FixtureTask, run: &ProcRun) -> bool {
    let a = &task.acceptance;
    if a.must_error == Some(true) && a.error_class.as_deref() == Some("timeout") {
        return run.timed_out;
    }
    if a.must_error == Some(true) {
        return false;
    }
    let mut ok = true;
    if let Some(code) = a.exit_code {
        ok = ok && run.status == Some(code);
    }
    if let Some(needle) = &a.stdout_contains {
        ok = ok && run.stdout.contains(needle);
    }
    if let Some(env_key) = &a.stdout_path_prefix_env {
        let prefix = std::env::var(env_key).unwrap_or_default();
        let out = run.stdout.trim();
        if prefix.is_empty() {
            return false;
        }
        let prefix_real = std::fs::canonicalize(&prefix)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| prefix.clone());
        let out_real = std::fs::canonicalize(out)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| out.to_string());
        ok = ok && out_real.starts_with(&prefix_real);
    }
    ok
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn schema_rejects_unknown() {
        let err = load_fixture_from_str(r#"{"schema_version":"nope","fixture_id":"x","tasks":[]}"#)
            .unwrap_err();
        assert!(matches!(err, FixtureError::Schema(_)));
    }

    fn load_fixture_from_str(s: &str) -> Result<DualHarnessFixture, FixtureError> {
        let fixture: DualHarnessFixture = serde_json::from_str(s)?;
        if fixture.schema_version != "pheno.dual_harness.fixture.v1" {
            return Err(FixtureError::Schema(fixture.schema_version));
        }
        Ok(fixture)
    }
}
