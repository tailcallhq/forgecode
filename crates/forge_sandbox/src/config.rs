//! Sandbox execution configuration.
//!
//! `SandboxConfig` is the input to [`crate::Backend::run`]. It describes the
//! command to execute and the constraints to enforce (filesystem rules,
//! network policy, timeout).
//!
//! Construct via [`SandboxConfig::builder`]:
//!
//! ```rust
//! use forge_sandbox::SandboxConfig;
//!
//! let config = SandboxConfig::builder()
//!     .command("cargo")
//!     .args(["test"])
//!     .working_dir("/tmp/proj")
//!     .build();
//! ```

use std::path::PathBuf;
use std::time::Duration;

/// What network access is allowed inside the sandbox.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum NetworkPolicy {
    /// No outbound network at all (most secure; default for code execution).
    #[default]
    DenyAll,
    /// Only loopback is reachable (for local services like a test database).
    LoopbackOnly,
    /// All egress allowed (use with care — agents can exfiltrate data).
    AllowAll,
    /// Specific hosts only (DNS names or IPs).
    AllowList(Vec<String>),
}

/// Filesystem access rule applied to a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesystemRule {
    /// Read-only access to the given path tree.
    ReadOnly(PathBuf),
    /// Read+write access to the given path tree.
    ReadWrite(PathBuf),
    /// Block all access to the given path (default for ~/.ssh, ~/.aws, etc.).
    Deny(PathBuf),
}

impl FilesystemRule {
    pub fn path(&self) -> &PathBuf {
        match self {
            FilesystemRule::ReadOnly(p)
            | FilesystemRule::ReadWrite(p)
            | FilesystemRule::Deny(p) => p,
        }
    }
}

/// Configuration for a single sandboxed execution.
#[derive(Debug, Clone, Default)]
pub struct SandboxConfig {
    /// Program to execute (must be on PATH or absolute).
    pub command: String,
    /// Arguments to pass.
    pub args: Vec<String>,
    /// Working directory for the process.
    pub working_dir: PathBuf,
    /// Environment variables to set (replaces inherited).
    pub env: Vec<(String, String)>,
    /// Filesystem access rules (applied in order; later rules win).
    pub filesystem: Vec<FilesystemRule>,
    /// Network policy.
    pub network: NetworkPolicy,
    /// Timeout for the execution (None = no timeout).
    pub timeout: Option<Duration>,
    /// Maximum stdout+stderr bytes captured (None = unbounded).
    pub output_limit: Option<usize>,
}

impl SandboxConfig {
    /// Start a new builder.
    pub fn builder() -> SandboxConfigBuilder {
        SandboxConfigBuilder::new()
    }

    /// Validate the config. Returns Ok if all fields are sane.
    pub fn validate(&self) -> Result<(), String> {
        if self.command.is_empty() {
            return Err("command must not be empty".to_string());
        }
        if self.command.contains('\0') {
            return Err("command contains NUL byte".to_string());
        }
        if !self.working_dir.exists() {
            return Err(format!(
                "working_dir does not exist: {}",
                self.working_dir.display()
            ));
        }
        for rule in &self.filesystem {
            let p = rule.path();
            if !p.exists() && !matches!(rule, FilesystemRule::ReadWrite(_)) {
                // Allow ReadWrite of non-existent paths (creating new dirs is fine).
                return Err(format!(
                    "filesystem rule path does not exist: {}",
                    p.display()
                ));
            }
        }
        if let Some(timeout) = self.timeout
            && timeout.is_zero()
        {
            return Err("timeout must be positive".to_string());
        }
        Ok(())
    }
}

/// Builder for [`SandboxConfig`].
#[derive(Debug, Default)]
pub struct SandboxConfigBuilder {
    inner: SandboxConfig,
}

impl SandboxConfigBuilder {
    pub fn new() -> Self {
        Self {
            inner: SandboxConfig {
                command: String::new(),
                args: Vec::new(),
                working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                env: Vec::new(),
                filesystem: Vec::new(),
                network: NetworkPolicy::default(),
                timeout: None,
                output_limit: None,
            },
        }
    }

    pub fn command(mut self, cmd: impl Into<String>) -> Self {
        self.inner.command = cmd.into();
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.inner.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.inner.args.push(arg.into());
        self
    }

    pub fn working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.inner.working_dir = dir.into();
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.inner.env.push((key.into(), value.into()));
        self
    }

    pub fn allow_read(mut self, path: impl Into<PathBuf>) -> Self {
        self.inner
            .filesystem
            .push(FilesystemRule::ReadOnly(path.into()));
        self
    }

    pub fn allow_read_write(mut self, path: impl Into<PathBuf>) -> Self {
        self.inner
            .filesystem
            .push(FilesystemRule::ReadWrite(path.into()));
        self
    }

    pub fn deny(mut self, path: impl Into<PathBuf>) -> Self {
        self.inner
            .filesystem
            .push(FilesystemRule::Deny(path.into()));
        self
    }

    pub fn network(mut self, policy: NetworkPolicy) -> Self {
        self.inner.network = policy;
        self
    }

    pub fn timeout(mut self, dur: Duration) -> Self {
        self.inner.timeout = Some(dur);
        self
    }

    pub fn output_limit(mut self, bytes: usize) -> Self {
        self.inner.output_limit = Some(bytes);
        self
    }

    pub fn build(self) -> SandboxConfig {
        self.inner
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn builder_produces_valid_config() {
        let working_dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let cfg = SandboxConfig::builder()
            .command("echo")
            .args(["hello"])
            .working_dir(working_dir.path())
            .build();
        assert_eq!(cfg.command, "echo");
        assert_eq!(cfg.args, vec!["hello"]);
        assert_eq!(cfg.working_dir, working_dir.path());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn empty_command_rejected() {
        let cfg = SandboxConfig::builder()
            .command("")
            .working_dir("/tmp")
            .build();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn nul_byte_rejected() {
        let cfg = SandboxConfig::builder()
            .command("echo\0bad")
            .working_dir("/tmp")
            .build();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn nonexistent_working_dir_rejected() {
        let cfg = SandboxConfig::builder()
            .command("echo")
            .working_dir("/nonexistent/path/xyz")
            .build();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn zero_timeout_rejected() {
        let cfg = SandboxConfig::builder()
            .command("echo")
            .working_dir("/tmp")
            .timeout(Duration::ZERO)
            .build();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn network_defaults_to_deny_all() {
        let cfg = SandboxConfig::builder().command("echo").build();
        assert_eq!(cfg.network, NetworkPolicy::DenyAll);
    }
}
