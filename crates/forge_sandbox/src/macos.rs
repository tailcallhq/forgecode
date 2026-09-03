//! macOS backend using Seatbelt via `sandbox-exec`.
//!
//! Seatbelt is the macOS analog of Landlock. We compile a `.sb` profile from
//! the user's `SandboxConfig`, write it to a temp file, and invoke
//! `sandbox-exec -f <profile> -- <command> <args>`.
//!
//! The `.sb` profile format is documented in `man sandbox-exec`.

use crate::backend::Backend;
use crate::config::{FilesystemRule, NetworkPolicy, SandboxConfig};
use crate::{SandboxError, SandboxOutput};
use async_trait::async_trait;
use bstr::ByteSlice;

#[derive(Clone)]
pub struct MacOsBackend {
    available: bool,
}

impl MacOsBackend {
    pub fn new() -> Result<Self, SandboxError> {
        let available = std::path::Path::new("/usr/bin/sandbox-exec").exists();
        Ok(Self { available })
    }

    /// Build a Seatbelt profile from the config.
    fn build_profile(config: &SandboxConfig) -> String {
        let mut profile = String::new();
        profile.push_str("(version 1)\n");
        profile.push_str("(deny default)\n");

        // Allow process execution / fork / exec.
        profile.push_str("(allow process-exec)\n");
        profile.push_str("(allow process-fork)\n");
        profile.push_str("(allow signal)\n");
        profile.push_str("(allow sysctl-read)\n");
        profile.push_str("(allow mach-lookup)\n");
        profile.push_str("(allow ipc-posix-shm-read*)\n");
        profile.push_str("(allow ipc-posix-shm-write*)\n");

        // Filesystem rules.
        for rule in &config.filesystem {
            let path = rule.path().to_string_lossy();
            match rule {
                FilesystemRule::ReadOnly(_) => {
                    profile.push_str(&format!("(allow file-read* (subpath \"{path}\"))\n"));
                }
                FilesystemRule::ReadWrite(_) => {
                    profile.push_str(&format!(
                        "(allow file-read* file-write* (subpath \"{path}\"))\n"
                    ));
                }
                FilesystemRule::Deny(_) => {
                    profile.push_str(&format!(
                        "(deny file-read* file-write* (subpath \"{path}\"))\n"
                    ));
                }
            }
        }

        // Network policy.
        match &config.network {
            NetworkPolicy::DenyAll => {
                profile.push_str("(deny network*)\n");
            }
            NetworkPolicy::LoopbackOnly => {
                profile.push_str("(allow network* (local ip \"localhost:*\"))\n");
                profile.push_str("(deny network* (remote ip))\n");
            }
            NetworkPolicy::AllowAll => {
                profile.push_str("(allow network*)\n");
            }
            NetworkPolicy::AllowList(hosts) => {
                profile.push_str("(allow network* (local ip))\n");
                profile.push_str("(allow network* (remote ip))\n");
                // Host allow-listing requires `(allow network-outbound (remote host "<host>"))` per host
                for host in hosts {
                    profile.push_str(&format!(
                        "(allow network-outbound (remote host \"{host}\"))\n"
                    ));
                }
            }
        }

        profile
    }
}

#[async_trait]
impl Backend for MacOsBackend {
    fn name(&self) -> &'static str {
        "macos-seatbelt"
    }

    fn enforces_isolation(&self) -> bool {
        self.available
    }

    async fn run(&self, config: &SandboxConfig) -> Result<SandboxOutput, SandboxError> {
        if !self.available {
            return Err(SandboxError::BackendUnavailable("macos-seatbelt"));
        }

        let profile = Self::build_profile(config);

        // Write profile to temp file.
        let mut tmp = tempfile::NamedTempFile::new()
            .map_err(|e| SandboxError::SetupFailed(format!("tempfile: {e}")))?;
        use std::io::Write;
        tmp.write_all(profile.as_bytes())
            .map_err(|e| SandboxError::SetupFailed(format!("write profile: {e}")))?;

        use tokio::process::Command;
        let mut cmd = Command::new("/usr/bin/sandbox-exec");
        cmd.arg("-f")
            .arg(tmp.path())
            .arg("--")
            .arg(&config.command)
            .args(&config.args)
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
            sandboxed: true,
        })
    }
}
