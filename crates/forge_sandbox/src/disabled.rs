//! Passthrough backend used when no real isolation backend is available.
//!
//! This is the safety net — when the kernel doesn't support Landlock (older
//! Linux), `sandbox-exec` is missing (rare macOS configuration), or the
//! Windows JobObject implementation hasn't landed yet, we fall back to plain
//! `tokio::process::Command` execution.
//!
//! The output is flagged with `sandboxed: false` so callers can see they
//! didn't get real isolation.

use crate::backend::Backend;
use crate::config::SandboxConfig;
use crate::{SandboxError, SandboxOutput};
use async_trait::async_trait;
use bstr::ByteSlice;

#[derive(Clone)]
pub struct DisabledBackend;

impl DisabledBackend {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Backend for DisabledBackend {
    fn name(&self) -> &'static str {
        "disabled"
    }

    fn enforces_isolation(&self) -> bool {
        false
    }

    async fn run(&self, config: &SandboxConfig) -> Result<SandboxOutput, SandboxError> {
        use tokio::process::Command;

        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args)
            .current_dir(&config.working_dir)
            .stdin(std::process::Stdio::null());

        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        let output = cmd.output().await?;
        Ok(SandboxOutput {
            stdout: output.stdout.to_str_lossy().into_owned(),
            stderr: output.stderr.to_str_lossy().into_owned(),
            exit_code: output.status.code().unwrap_or(-1),
            sandboxed: false,
        })
    }
}
