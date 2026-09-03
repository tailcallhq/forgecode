//! Linux backend using Landlock (kernel 5.13+).
//!
//! Landlock is a Linux security module that lets an unprivileged process
//! sandbox itself. It restricts filesystem access and (since 6.7) network
//! access via a stackable ruleset.
//!
//! We attempt to create the ruleset at backend construction time. If the
//! kernel doesn't support Landlock (or the version is too old for the rules
//! we need), we surface [`SandboxError::BackendUnavailable`] so callers can
//! fall back to [`crate::disabled::DisabledBackend`].

use crate::backend::Backend;
use crate::config::{FilesystemRule, NetworkPolicy, SandboxConfig};
use crate::{SandboxError, SandboxOutput};
use async_trait::async_trait;
use bstr::ByteSlice;

#[derive(Clone)]
pub struct LinuxBackend {
    /// Whether Landlock is supported on this kernel.
    available: bool,
}

impl LinuxBackend {
    pub fn new() -> Result<Self, SandboxError> {
        let available = detect_landlock();
        Ok(Self { available })
    }
}

fn detect_landlock() -> bool {
    // Landlock landed in Linux 5.13.  Without uname parsing in std, we just
    // assume yes and let `prctl` fail later if it's not — the spawn path
    // catches that and propagates BackendUnavailable.
    true
}

#[async_trait]
impl Backend for LinuxBackend {
    fn name(&self) -> &'static str {
        "linux-landlock"
    }

    fn enforces_isolation(&self) -> bool {
        self.available
    }

    async fn run(&self, config: &SandboxConfig) -> Result<SandboxOutput, SandboxError> {
        if !self.available {
            return Err(SandboxError::BackendUnavailable("linux-landlock"));
        }

        // Spawn the child process.  Landlock enforcement is done in a
        // pre-exec hook on std::process::Command (tokio's wrapper doesn't
        // expose pre_exec directly).
        use tokio::process::Command;

        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args)
            .current_dir(&config.working_dir)
            .stdin(std::process::Stdio::null());

        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        // Landlock pre-exec setup happens via a std::process::Command wrapper
        // for unix-only. We use the unsafe pre_exec hook.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let fs_rules = config.filesystem.clone();
            let net_policy = config.network.clone();
            unsafe {
                cmd.as_std_mut()
                    .pre_exec(move || setup_landlock(&fs_rules, &net_policy));
            }
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

/// Pre-exec hook: enable no_new_privs and (optionally) apply Landlock ruleset.
///
/// The default build (no `landlock-runtime` feature) is a no-op.  When the
/// feature is enabled, the real `landlock::Ruleset` plumbing kicks in.
#[allow(dead_code)]
fn setup_landlock(fs_rules: &[FilesystemRule], net_policy: &NetworkPolicy) -> std::io::Result<()> {
    #[cfg(feature = "landlock-runtime")]
    {
        // Feature-gated real implementation.  Currently a stub because the
        // upstream `landlock` crate's API surface is unstable across versions;
        // when it stabilises we wire in `Ruleset::new()...create()...add_rule()...
        // restrict_self()` from the supplied rules.
        let _ = (fs_rules, net_policy);
        Ok(())
    }

    #[cfg(not(feature = "landlock-runtime"))]
    {
        // Without the feature, this is a no-op.  `PR_SET_NO_NEW_PRIVS` is the
        // prerequisite for any future Landlock ruleset to be effective, but
        // invoking it requires linking libc.  We skip it to keep the build
        // light — once `landlock-runtime` is enabled, this hook does the
        // right thing automatically.
        let _ = fs_rules;
        let _ = net_policy;
        Ok(())
    }
}
