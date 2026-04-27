#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd};

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

use async_trait::async_trait;
use forge_domain::{Agent, CachedHook, ModelId, ToolCallFull, ToolCallInterceptor};
use serde::{Deserialize, Serialize};
use tracing::debug;

// ---------------------------------------------------------------------------
// PreparedHook — pre-created executable, kept alive for the entire session
// ---------------------------------------------------------------------------

/// A hook whose executable form is determined by platform.
///
/// On Linux, a sealed memfd is created once at construction and reused for
/// every `spawn()` call. On non-Linux (macOS, Windows, etc.), no pre-created
/// executable exists — the original source file is spawned directly after
/// verifying its content hash matches the cached version.
enum Executable {
    /// Linux: `/proc/self/fd/<n>` backed by a sealed memfd. The `Memfd` guard
    /// keeps the fd alive — it must NOT be dropped until the interceptor is
    /// dropped (which happens at session end, long after any child has exec'd).
    #[cfg(target_os = "linux")]
    Memfd { path: PathBuf, _guard: Memfd },
    /// Non-Linux: the original source path. Content hash is verified before
    /// each spawn to mitigate TOCTOU attacks.
    #[cfg(not(target_os = "linux"))]
    Source { expected_hash: String },
}

/// A fully-prepared hook ready for execution.
///
/// On Linux, a sealed memfd is created once at construction and reused for
/// every `spawn()` call. On non-Linux, the original source file is spawned
/// directly after verifying its content hash matches the cached version.
struct PreparedHook {
    /// Original source path — kept for logging/diagnostics and used as the
    /// executable path on non-Linux.
    source: PathBuf,
    /// Pre-created executable (memfd on Linux) or hash-verified source path
    /// (non-Linux).
    executable: Executable,
}

impl PreparedHook {
    /// Prepares an executable from cached hook content.
    ///
    /// On Linux, creates a sealed memfd and returns `/proc/self/fd/<n>`.
    /// On non-Linux, stores the expected content hash for verification at
    /// spawn time.
    fn prepare(hook: CachedHook) -> std::io::Result<Self> {
        let source = hook.source().to_path_buf();
        let executable = prepare_executable(&hook)?;
        Ok(Self { source, executable })
    }

    /// Spawns the hook script as a child process.
    ///
    /// On Linux, reuses the pre-created memfd path. On non-Linux, verifies
    /// the on-disk content hash matches the cached version before spawning
    /// the original source file directly.
    fn spawn(&self) -> std::io::Result<tokio::process::Child> {
        let exe_path = match &self.executable {
            #[cfg(target_os = "linux")]
            Executable::Memfd { path, .. } => path.clone(),
            #[cfg(not(target_os = "linux"))]
            Executable::Source { expected_hash } => {
                let actual_hash =
                    super::trust::compute_file_hash(&self.source).map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!(
                                "Hook integrity check failed for {}: {}",
                                self.source.display(),
                                e
                            ),
                        )
                    })?;
                if actual_hash != *expected_hash {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "Hook integrity check failed for {}: content hash mismatch (expected {}, got {})",
                            self.source.display(),
                            expected_hash,
                            actual_hash
                        ),
                    ));
                }
                self.source.clone()
            }
        };
        Command::new(exe_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
    }
}

/// Prepares an executable from cached hook content.
///
/// On Linux: creates a `memfd`, writes content, seals it, and returns
/// `/proc/self/fd/<n>` for execution. The `Memfd` guard keeps the file
/// descriptor alive for the entire session.
///
/// On non-Linux: computes the content hash and returns a `Source` variant
/// that will verify integrity before each spawn.
#[cfg(target_os = "linux")]
fn prepare_executable(hook: &CachedHook) -> std::io::Result<Executable> {
    let name = hook
        .source()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "hook".to_string());

    // Create an anonymous in-memory file descriptor
    let guard = memfd_create(&name)?;
    let raw_fd = guard.as_raw_fd();

    // Write the cached content using libc write to avoid IO safety issues
    let content = hook.content();
    let mut offset = 0;
    while offset < content.len() {
        let written = unsafe {
            libc::write(
                raw_fd,
                content[offset..].as_ptr() as *const libc::c_void,
                content.len() - offset,
            )
        };
        if written < 0 {
            return Err(std::io::Error::last_os_error());
        }
        offset += written as usize;
    }

    // Set executable permission so the kernel allows execve via /proc/self/fd/N
    let ret = unsafe { libc::fchmod(raw_fd, 0o700) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }

    // Seal the memfd so the content cannot be modified
    seal_memfd(raw_fd)?;

    // Return /proc/self/fd/N path for execution, keeping the Memfd alive
    let path = PathBuf::from(format!("/proc/self/fd/{raw_fd}"));
    Ok(Executable::Memfd { path, _guard: guard })
}

#[cfg(not(target_os = "linux"))]
fn prepare_executable(hook: &CachedHook) -> std::io::Result<Executable> {
    let expected_hash = super::trust::compute_file_hash(hook.source())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    Ok(Executable::Source { expected_hash })
}

// ---------------------------------------------------------------------------
// Linux memfd helpers
// ---------------------------------------------------------------------------

/// RAII wrapper around a memfd file descriptor.
#[cfg(target_os = "linux")]
struct Memfd(std::os::fd::OwnedFd);

#[cfg(target_os = "linux")]
impl Memfd {
    /// Takes ownership of the raw fd and closes it on drop.
    fn new(fd: std::os::fd::OwnedFd) -> Self {
        Self(fd)
    }
}

#[cfg(target_os = "linux")]
impl std::os::fd::AsFd for Memfd {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.0.as_fd()
    }
}

#[cfg(target_os = "linux")]
impl Memfd {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.0.as_raw_fd()
    }
}

/// Creates a memfd and returns a RAII wrapper.
#[cfg(target_os = "linux")]
fn memfd_create(name: &str) -> std::io::Result<Memfd> {
    use std::ffi::CString;

    let c_name = CString::new(name).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "memfd name contains NUL")
    })?;

    // MFD_ALLOW_SEALING = 0x0002U
    // NOTE: We intentionally do NOT set MFD_CLOEXEC (0x0001). When the kernel
    // processes a shebang (e.g. #!/bin/bash), it opens the script path again
    // from the interpreter. If the fd had CLOEXEC, it would be closed after
    // execve and the interpreter would fail to read the script.
    let flags: libc::c_uint = 0x0002;

    // SAFETY: memfd_create is a Linux system call. `c_name` is a valid
    // null-terminated C string and flags are valid constants.
    let fd = unsafe { libc::memfd_create(c_name.as_ptr(), flags) };

    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }

    // SAFETY: fd is a valid, owned file descriptor returned by memfd_create.
    Ok(Memfd::new(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) }))
}

/// Applies sealing to prevent further modifications to the memfd content.
#[cfg(target_os = "linux")]
fn seal_memfd(fd: std::os::fd::RawFd) -> std::io::Result<()> {
    // F_SEAL_SEAL | F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_WRITE
    const SEALS: libc::c_ulong = 0x0001 | 0x0002 | 0x0004 | 0x0008;

    // SAFETY: fcntl with F_ADD_SEALS is safe for valid memfd file descriptors.
    let ret = unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, SEALS) };

    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Interceptor
// ---------------------------------------------------------------------------

/// Interceptor that executes external hook scripts to modify tool calls.
///
/// At construction time, the full content of every hook script is read into
/// memory. On Linux, each hook is pre-compiled into a sealed memfd for
/// zero-TOCTOU execution. On non-Linux, the content hash is stored and
/// verified before each spawn to detect tampering.
///
/// At runtime, `intercept()` spawns each executable — zero per-call memfd
/// overhead on Linux, hash-verified direct execution on non-Linux.
///
/// # Hook protocol
///
/// Input (stdin):
/// ```json
/// {"tool_name": "shell", "tool_input": {"command": "git status"}}
/// ```
///
/// Output (stdout) -- allow with modification:
/// ```json
/// {"decision": "allow", "hookSpecificOutput": {"tool_input": {"command": "rtk git status"}}}
/// ```
///
/// Output (stdout) -- allow without modification:
/// ```json
/// {"decision": "allow"}
/// ```
///
/// Output (stdout) -- deny:
/// ```json
/// {"decision": "deny", "reason": "blocked by policy"}
/// ```
#[derive(Clone)]
pub struct ExternalHookInterceptor {
    hooks: std::sync::Arc<Vec<PreparedHook>>,
    timeout_secs: u64,
}

impl ExternalHookInterceptor {
    /// Creates a new external hook interceptor from cached hook content.
    ///
    /// Each hook's content is pre-compiled into an executable (memfd on Linux,
    /// temp-file elsewhere) at construction time. The prepared executables are
    /// reused for every `intercept()` call.
    ///
    /// # Arguments
    ///
    /// * `cached_hooks` - Hook scripts whose content has been read into memory
    /// * `timeout_secs` - Optional timeout in seconds for hook execution (default: 30)
    pub fn new(
        cached_hooks: std::sync::Arc<Vec<CachedHook>>,
        timeout_secs: Option<u64>,
    ) -> Self {
        let hooks: Vec<PreparedHook> = cached_hooks
            .iter()
            .filter_map(|hook| match PreparedHook::prepare(hook.clone()) {
                Ok(prepared) => Some(prepared),
                Err(e) => {
                    tracing::warn!(
                        hook = %hook.source().display(),
                        error = %e,
                        "Failed to prepare hook executable, skipping"
                    );
                    None
                }
            })
            .collect();

        Self {
            hooks: std::sync::Arc::new(hooks),
            timeout_secs: timeout_secs.unwrap_or(30),
        }
    }

    /// Run a single prepared hook script, piping JSON input and parsing JSON
    /// output.
    async fn run_hook(
        hook: &PreparedHook,
        input: &HookInput,
        timeout_secs: u64,
    ) -> anyhow::Result<HookOutput> {
        let input_json = serde_json::to_string(input)?;

        debug!(hook = %hook.source.display(), "Executing external hook");

        let mut child = match hook.spawn() {
            Ok(c) => c,
            Err(e) => {
                debug!(
                    hook = %hook.source.display(),
                    error = %e,
                    "Failed to spawn hook, treating as allow"
                );
                return Ok(HookOutput {
                    decision: "allow".to_string(),
                    reason: None,
                    hook_specific_output: None,
                });
            }
        };

        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(input_json.as_bytes()).await?;
        drop(stdin);

        // Wait for output with timeout
        let output = match timeout(Duration::from_secs(timeout_secs), child.wait_with_output())
            .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => {
                debug!(
                    hook = %hook.source.display(),
                    timeout_secs = timeout_secs,
                    "Hook execution timed out, treating as allow"
                );
                return Ok(HookOutput {
                    decision: "allow".to_string(),
                    reason: Some("Hook execution timed out".to_string()),
                    hook_specific_output: None,
                });
            }
        };

        // Log stderr at debug level for hook debugging
        if !output.stderr.is_empty() {
            let stderr_str = String::from_utf8_lossy(&output.stderr);
            debug!(
                hook = %hook.source.display(),
                stderr = %stderr_str,
                "Hook stderr output"
            );
        }

        if !output.status.success() {
            debug!(
                hook = %hook.source.display(),
                exit_code = ?output.status.code(),
                "Hook exited with non-zero status, skipping"
            );
            // Treat non-zero exit as "allow" (pass-through)
            return Ok(HookOutput {
                decision: "allow".to_string(),
                reason: None,
                hook_specific_output: None,
            });
        }

        let output_str = String::from_utf8_lossy(&output.stdout);
        match serde_json::from_str::<HookOutput>(&output_str) {
            Ok(hook_output) => Ok(hook_output),
            Err(e) => {
                debug!(
                    hook = %hook.source.display(),
                    error = %e,
                    "Hook output was not valid JSON, treating as allow"
                );
                Ok(HookOutput {
                    decision: "allow".to_string(),
                    reason: None,
                    hook_specific_output: None,
                })
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct HookInput {
    tool_name: String,
    tool_input: serde_json::Value,
}

#[derive(Serialize, Deserialize, Clone)]
struct HookOutput {
    decision: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: Option<HookSpecificOutput>,
}

#[derive(Serialize, Deserialize, Clone)]
struct HookSpecificOutput {
    tool_input: serde_json::Value,
}

/// Returns the sorted list of hook scripts for a given event.
///
/// Scans `~/.forge/hooks/<event>.d/` for executable files, sorted
/// alphabetically by filename.
///
/// This is used by the startup loader and CLI commands. It is not called
/// during `intercept()` — the interceptor uses cached content instead.
pub fn discover_hooks(event_name: &str) -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let hook_dir = home
        .join(".forge")
        .join("hooks")
        .join(format!("{event_name}.d"));

    if !hook_dir.is_dir() {
        return Vec::new();
    }

    let Ok(entries) = std::fs::read_dir(&hook_dir) else {
        return Vec::new();
    };

    let mut hooks: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            // Only include files (not directories)
            path.is_file()
        })
        .filter(|path| {
            // On Unix, only include files with the executable bit set
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(path)
                    .map(|m| m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
            }
            // On non-Unix, include all regular files — the OS determines
            // executability via file association at spawn time
            #[cfg(not(unix))]
            {
                let _ = path;
                true
            }
        })
        .collect();

    // Sort alphabetically for deterministic execution order
    hooks.sort();
    hooks
}

#[async_trait]
impl ToolCallInterceptor for ExternalHookInterceptor {
    async fn intercept(
        &self,
        tool_call: &mut ToolCallFull,
        _agent: &Agent,
        _model_id: &ModelId,
    ) -> anyhow::Result<()> {
        let hooks = &self.hooks;
        if hooks.is_empty() {
            return Ok(());
        }

        // Build initial input from the tool call
        let mut current_input = HookInput {
            tool_name: tool_call.name.as_str().to_string(),
            tool_input: serde_json::to_value(&tool_call.arguments)?,
        };

        for hook in hooks.iter() {
            let output = Self::run_hook(hook, &current_input, self.timeout_secs).await?;

            match output.decision.as_str() {
                "deny" => {
                    let reason = output.reason.as_deref().unwrap_or("no reason provided");
                    debug!(
                        hook = %hook.source.display(),
                        reason = reason,
                        "Hook denied tool call"
                    );
                    return Err(anyhow::anyhow!("Hook denied tool call: {}", reason));
                }
                "allow" => {
                    if let Some(specific) = &output.hook_specific_output {
                        // Hook modified the tool input -- update for next hook
                        // in the pipeline
                        current_input.tool_input = specific.tool_input.clone();
                    }
                    // Continue to next hook
                }
                other => {
                    debug!(
                        hook = %hook.source.display(),
                        decision = other,
                        "Unknown hook decision, treating as allow"
                    );
                }
            }
        }

        // Apply the final result back to the tool call
        if let Ok(updated_args) =
            serde_json::from_value::<forge_domain::ToolCallArguments>(current_input.tool_input)
        {
            tool_call.arguments = updated_args;
        } else {
            debug!("Failed to deserialize hook output, keeping original arguments");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_hooks_empty_dir() {
        // This test just ensures the function doesn't panic with missing dirs
        let hooks = discover_hooks("nonexistent-event");
        assert!(hooks.is_empty());
    }

    #[test]
    fn test_hook_output_deserialize_allow() {
        let json = r#"{"decision":"allow"}"#;
        let output: HookOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.decision, "allow");
        assert!(output.hook_specific_output.is_none());
    }

    #[test]
    fn test_hook_output_deserialize_allow_with_modification() {
        let json =
            r#"{"decision":"allow","hookSpecificOutput":{"tool_input":{"command":"rtk ls"}}}"#;
        let output: HookOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.decision, "allow");
        assert!(output.hook_specific_output.is_some());
        let specific = output.hook_specific_output.unwrap();
        assert_eq!(specific.tool_input["command"], "rtk ls");
    }

    #[test]
    fn test_hook_output_deserialize_deny() {
        let json = r#"{"decision":"deny","reason":"blocked"}"#;
        let output: HookOutput = serde_json::from_str(json).unwrap();
        assert_eq!(output.decision, "deny");
        assert_eq!(output.reason.as_deref(), Some("blocked"));
    }

    #[test]
    fn test_interceptor_with_empty_cached_hooks() {
        let interceptor =
            ExternalHookInterceptor::new(std::sync::Arc::new(Vec::new()), None);
        assert!(interceptor.hooks.is_empty());
    }

    #[test]
    fn test_cached_hook_from_path() {
        let dir = tempfile::tempdir().unwrap();
        let hook_path = dir.path().join("test_hook.sh");
        std::fs::write(&hook_path, "#!/bin/bash\necho hello").unwrap();

        let cached = CachedHook::from_path(hook_path.clone()).unwrap();
        assert_eq!(cached.source(), hook_path);
        assert_eq!(cached.content(), b"#!/bin/bash\necho hello");
    }

    #[tokio::test]
    async fn test_interceptor_deny_tool_call() {
        // Create a temporary hook script that outputs deny
        let dir = tempfile::tempdir().unwrap();
        let hook_path = dir.path().join("test_hook.sh");
        std::fs::write(
            &hook_path,
            r#"#!/bin/bash
echo '{"decision":"deny","reason":"test block"}'
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let cached = CachedHook::from_path(hook_path).unwrap();
        let interceptor =
            ExternalHookInterceptor::new(std::sync::Arc::new(vec![cached]), None);
        let mut tool_call = forge_domain::ToolCallFull::new("test_tool")
            .arguments(forge_domain::ToolCallArguments::from(serde_json::json!({})));
        // Create minimal agent for test (agent parameter is unused in intercept)
        let agent = forge_domain::Agent::new(
            forge_domain::AgentId::from("test-agent"),
            "test-provider".to_string().into(),
            forge_domain::ModelId::from("test-model"),
        );
        let model_id = forge_domain::ModelId::new("test-model");

        let result = interceptor.intercept(&mut tool_call, &agent, &model_id).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("denied tool call"));
        assert!(err.to_string().contains("test block"));
    }

    #[tokio::test]
    async fn test_hook_timeout_cleans_up_process() {
        // Create a hook that runs longer than the timeout
        let dir = tempfile::tempdir().unwrap();
        let hook_path = dir.path().join("slow_hook.sh");
        std::fs::write(&hook_path, "#!/bin/bash\nsleep 10").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let cached = CachedHook::from_path(hook_path).unwrap();
        // Pass timeout as injected parameter (1 second) instead of using env var
        let interceptor =
            ExternalHookInterceptor::new(std::sync::Arc::new(vec![cached]), Some(1));
        let mut tool_call = forge_domain::ToolCallFull::new("test_tool")
            .arguments(forge_domain::ToolCallArguments::from(serde_json::json!({})));
        let agent = forge_domain::Agent::new(
            forge_domain::AgentId::from("test-agent"),
            "test-provider".to_string().into(),
            forge_domain::ModelId::from("test-model"),
        );
        let model_id = forge_domain::ModelId::new("test-model");

        let start = std::time::Instant::now();
        let result = interceptor.intercept(&mut tool_call, &agent, &model_id).await;
        let duration = start.elapsed();

        // Verify timeout occurred (should take ~1 second, not 10)
        assert!(duration < std::time::Duration::from_secs(5));
        // Timeout should result in Ok(()) (treated as allow)
        assert!(result.is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_memfd_create_and_seal() {
        let fd = memfd_create("test-hook").expect("memfd_create should succeed");
        let raw = fd.as_raw_fd();

        // Write some data using libc write (borrows the fd, doesn't take ownership)
        let data = b"#!/bin/bash\necho hello";
        let written =
            unsafe { libc::write(raw, data.as_ptr() as *const libc::c_void, data.len()) };
        assert!(written >= 0, "write should succeed");

        // Seal it
        seal_memfd(raw).expect("seal_memfd should succeed");

        // Further writes should fail (sealed)
        let more_data = b"more data";
        let result =
            unsafe { libc::write(raw, more_data.as_ptr() as *const libc::c_void, more_data.len()) };
        assert!(result < 0, "Write after seal should fail");
    }
}
