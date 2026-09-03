// forge_daemon — Rust orchestration layer over the Zig kqueue+posix_spawn
// daemon.
#![allow(dead_code)] // FFI symbols are used by tests and external callers
//
// Split: Zig = hot core (kqueue/posix_spawn/socket), Rust =
// config/IPC/observability. The Zig core is compiled to libforge_daemon_core.a
// (C ABI) by build.rs.
//
// Two usage modes:
//
//   1. In-process C-ABI dispatch (DaemonDispatch): Calls
//      forge_daemon_dispatch() directly from the same process — the Zig core
//      handles posix_spawn, pipe, waitpid.  No daemon socket needed.
//
//   2. Socket-based client (DaemonClient): Connects to a running forge-daemon
//      process over a Unix socket. Sends JSON requests, receives JSON
//      responses.
//
// The Rust side of forge_main can use either mode; mode 1 is simpler for
// single-machine use.  Mode 2 supports the warm-pool long-running daemon
// model that eliminates dyld+tokio init cost across multiple callers.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(unix)]
use tokio::net::UnixStream;
#[cfg(forge_daemon_zig_core)]
use tracing::debug;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// FFI bindings to libforge_daemon_core.a
// ---------------------------------------------------------------------------

#[cfg(forge_daemon_zig_core)]
unsafe extern "C" {
    /// Start the daemon (bind socket, init kqueue).
    /// socket_path: NUL-terminated C string; null →
    /// /tmp/forge-daemon-<uid>.sock Returns 0 on success, -1 on error.
    fn forge_daemon_start(socket_path: *const std::os::raw::c_char) -> std::os::raw::c_int;

    /// Stop the daemon (close socket, kill workers).
    fn forge_daemon_stop();

    /// Returns 1 if daemon is running, 0 otherwise.
    fn forge_daemon_is_running() -> std::os::raw::c_int;

    /// Write the active socket path into `out` (capacity `cap`,
    /// NUL-terminated). Returns bytes written (excl. NUL), or -1 if not
    /// started.
    fn forge_daemon_socket_path(out: *mut std::os::raw::c_char, cap: usize) -> std::os::raw::c_int;

    /// Dispatch one forge task via posix_spawn (hot path).
    /// Returns exit code; -1 on spawn failure.
    fn forge_daemon_dispatch(
        forge_bin: *const std::os::raw::c_char,
        prompt: *const std::os::raw::c_char,
        model: *const std::os::raw::c_char,
        cwd: *const std::os::raw::c_char,
        result_buf: *mut std::os::raw::c_char,
        result_cap: usize,
    ) -> std::os::raw::c_int;
}

#[cfg(unix)]
unsafe extern "C" {
    fn getuid() -> u32;
}

// ---------------------------------------------------------------------------
// Mode 1: In-process dispatch (no daemon socket required)
// ---------------------------------------------------------------------------

/// Synchronous in-process dispatch via the Zig C-ABI posix_spawn hot path.
/// Eliminates the ~47ms dyld+tokio init cost per spawn (#74).
pub struct DaemonDispatch;

impl DaemonDispatch {
    /// Dispatch a single forge task and return its stdout output + exit code.
    #[cfg(forge_daemon_zig_core)]
    pub fn dispatch(
        forge_bin: &str,
        prompt: &str,
        model: &str,
        cwd: &Path,
    ) -> Result<(i32, Vec<u8>)> {
        use std::ffi::CString;

        let forge_bin_c = CString::new(forge_bin).context("forge_bin NUL")?;
        let prompt_c = CString::new(prompt).context("prompt NUL")?;
        let model_c = CString::new(model).context("model NUL")?;
        let cwd_c = CString::new(cwd.to_str().context("cwd UTF-8")?).context("cwd NUL")?;

        let mut result_buf = vec![0u8; 65536];
        let exit_code = unsafe {
            forge_daemon_dispatch(
                forge_bin_c.as_ptr(),
                prompt_c.as_ptr(),
                model_c.as_ptr(),
                cwd_c.as_ptr(),
                result_buf.as_mut_ptr() as *mut std::os::raw::c_char,
                result_buf.len(),
            )
        };

        // Find the NUL terminator to get the actual output length.
        let nul_pos = result_buf
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(result_buf.len());
        result_buf.truncate(nul_pos);

        debug!(
            exit_code,
            output_bytes = nul_pos,
            "forge_daemon_dispatch returned"
        );
        Ok((exit_code, result_buf))
    }

    /// The Zig daemon core uses macOS kqueue/posix_spawn internals and is not
    /// linked into non-macOS builds. Keep workspace checks green while making
    /// platform support explicit at the call boundary.
    #[cfg(not(forge_daemon_zig_core))]
    pub fn dispatch(
        _forge_bin: &str,
        _prompt: &str,
        _model: &str,
        _cwd: &Path,
    ) -> Result<(i32, Vec<u8>)> {
        anyhow::bail!(
            "forge daemon dispatch requires FORGE_DAEMON_BUILD_ZIG_CORE=1 on supported macOS hosts"
        )
    }
}

// ---------------------------------------------------------------------------
// Mode 2: Socket-based client
// ---------------------------------------------------------------------------

/// Wire request sent to the daemon over Unix socket.
#[derive(Debug, Serialize)]
pub struct DaemonRequest {
    pub id: u64,
    pub op: String, // "run" | "ping" | "shutdown"
    #[serde(skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cwd: String,
}

/// Wire response from the daemon.
#[derive(Debug, Deserialize)]
pub struct DaemonResponse {
    pub id: Option<u64>,
    pub status: String, // "ok" | "err" | "pong"
    pub exit_code: Option<i32>,
    pub output_len: Option<u64>,
}

/// Async client connecting to a running forge-daemon over a Unix socket.
pub struct DaemonClient {
    socket_path: String,
    next_id: u64,
}

impl DaemonClient {
    /// Create a client for the given socket path.
    pub fn new(socket_path: impl Into<String>) -> Self {
        Self { socket_path: socket_path.into(), next_id: 1 }
    }

    /// Create a client using the default socket path
    /// (/tmp/forge-daemon-<uid>.sock).
    pub fn default_path() -> Self {
        let uid = libc_getuid();
        Self::new(format!("/tmp/forge-daemon-{uid}.sock"))
    }

    /// Ping the daemon; returns true if alive.
    pub async fn ping(&mut self) -> bool {
        let id = self.next_id();
        self.send_recv(&DaemonRequest {
            id,
            op: "ping".into(),
            prompt: String::new(),
            model: String::new(),
            cwd: String::new(),
        })
        .await
        .map(|r| r.status == "pong")
        .unwrap_or(false)
    }

    /// Send a task to the daemon and wait for the response.
    pub async fn run(&mut self, prompt: &str, model: &str, cwd: &str) -> Result<DaemonResponse> {
        let id = self.next_id();
        self.send_recv(&DaemonRequest {
            id,
            op: "run".into(),
            prompt: prompt.to_owned(),
            model: model.to_owned(),
            cwd: cwd.to_owned(),
        })
        .await
    }

    /// Request daemon shutdown.
    pub async fn shutdown(&mut self) -> Result<()> {
        let id = self.next_id();
        let _ = self
            .send_recv(&DaemonRequest {
                id,
                op: "shutdown".into(),
                prompt: String::new(),
                model: String::new(),
                cwd: String::new(),
            })
            .await;
        Ok(())
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    #[cfg(unix)]
    async fn send_recv(&self, req: &DaemonRequest) -> Result<DaemonResponse> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .with_context(|| format!("connect to daemon at {}", self.socket_path))?;

        let payload = serde_json::to_vec(req).context("serialize request")?;
        let len = payload.len() as u32;
        stream
            .write_all(&len.to_le_bytes())
            .await
            .context("write len")?;
        stream.write_all(&payload).await.context("write payload")?;

        let mut len_buf = [0u8; 4];
        stream
            .read_exact(&mut len_buf)
            .await
            .context("read response len")?;
        let resp_len = u32::from_le_bytes(len_buf) as usize;

        let mut resp_buf = vec![0u8; resp_len];
        stream
            .read_exact(&mut resp_buf)
            .await
            .context("read response")?;

        serde_json::from_slice(&resp_buf).context("deserialize response")
    }

    #[cfg(not(unix))]
    async fn send_recv(&self, _req: &DaemonRequest) -> Result<DaemonResponse> {
        anyhow::bail!("forge daemon socket client requires Unix domain sockets")
    }
}

// ---------------------------------------------------------------------------
// Daemon process management (start/stop the standalone forge-daemon binary)
// ---------------------------------------------------------------------------

/// Guard that stops the Zig daemon on drop.
pub struct DaemonGuard {
    child: tokio::process::Child,
    socket_path: String,
}

impl DaemonGuard {
    /// Launch the standalone forge-daemon binary; wait until the socket
    /// appears.
    pub async fn start(daemon_bin: &Path, forge_bin: &Path) -> Result<Self> {
        let socket_path = format!("/tmp/forge-daemon-{}.sock", libc_getuid());

        info!(daemon_bin = %daemon_bin.display(), %socket_path, "starting forge-daemon");

        let child = tokio::process::Command::new(daemon_bin)
            .env("FORGE_DAEMON_SOCKET", &socket_path)
            .env("FORGE_BIN", forge_bin)
            .spawn()
            .with_context(|| format!("spawn {}", daemon_bin.display()))?;

        // Wait up to 2s for the socket to appear.
        for _ in 0..20 {
            if Path::new(&socket_path).exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        if !Path::new(&socket_path).exists() {
            warn!(%socket_path, "daemon socket did not appear within 2s");
        }

        Ok(Self { child, socket_path })
    }

    pub fn socket_path(&self) -> &str {
        &self.socket_path
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        // Best-effort SIGTERM on drop.
        let _ = self.child.start_kill();
    }
}

#[inline]
fn libc_getuid() -> u32 {
    #[cfg(unix)]
    unsafe {
        getuid()
    }

    #[cfg(not(unix))]
    {
        0
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, forge_daemon_zig_core))]
mod tests {
    use super::*;

    #[test]
    fn daemon_not_running_initially() {
        // forge_daemon_is_running() returns 0 when forge_daemon_start() has
        // not been called yet in this process.
        let running = unsafe { forge_daemon_is_running() };
        // Note: if another test called start(), this may be 1.  Assert both
        // values are valid C_int values (0 or 1).
        assert!(running == 0 || running == 1);
    }

    #[test]
    fn daemon_start_stop_lifecycle() {
        // Start daemon on a tmp socket path.
        let path = format!("/tmp/forge-daemon-test-{}.sock", std::process::id());
        let path_c = CString::new(path.clone()).unwrap();
        let ret = unsafe { forge_daemon_start(path_c.as_ptr()) };
        assert_eq!(ret, 0, "forge_daemon_start failed");
        assert_eq!(unsafe { forge_daemon_is_running() }, 1);

        // Socket file should exist.
        assert!(
            Path::new(&path).exists(),
            "socket file not created at {path}"
        );

        // Query socket path back.
        let mut buf = [0i8; 256];
        let n = unsafe { forge_daemon_socket_path(buf.as_mut_ptr(), buf.len()) };
        assert!(n > 0, "forge_daemon_socket_path returned {n}");
        let got = unsafe {
            std::ffi::CStr::from_ptr(buf.as_ptr())
                .to_str()
                .unwrap()
                .to_owned()
        };
        assert_eq!(got, path);

        unsafe { forge_daemon_stop() };
        assert_eq!(unsafe { forge_daemon_is_running() }, 0);
        // Socket file should be cleaned up.
        assert!(
            !Path::new(&path).exists(),
            "socket file not removed after stop"
        );
    }
}
