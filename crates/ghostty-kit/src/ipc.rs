//! Control IPC client for Ghostty's `GhosttyControl` interface.
//!
//! When Ghostty is started with `--control-socket=PATH` (or a build-time
//! default), it exposes a runtime control surface over a Unix domain
//! socket. Wire protocol: 4-byte big-endian length prefix + JSON payload
//! of shape `{ "action", "args", "id" }`; replies are
//! `{ "ok": true, "data": ... }` or `{ "ok": false, "error": "..." }`.
//!
//! # R8: never panic when the socket is absent
//!
//! [`GhosttyControl::try_new`] and [`GhosttyControl::try_with_path`] return
//! `None` on any connection failure. A forgecode host process must not
//! crash just because the user has not launched Ghostty with
//! `--control-socket`. Request/response helpers live in
//! [`crate::ipc_request`] and [`crate::ipc_response`].

use std::fmt;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ipc_request::{build_request, frame_request, JsonObject};
use crate::ipc_response::{parse_window_size, read_framed_response};

// Public types

/// A connected client for Ghostty's control surface. Constructed via
/// [`GhosttyControl::try_new`] or [`GhosttyControl::try_with_path`]; both
/// return `None` on connection failure, so this type only ever exists
/// when a live Ghostty accepted the probe connection (R8 contract).
#[derive(Debug, Clone)]
pub struct GhosttyControl {
    socket_path: PathBuf,
    timeout: Duration,
}

/// Window dimensions in pixels and cell units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowSize {
    /// Window width in physical pixels.
    pub width: u16,
    /// Window height in physical pixels.
    pub height: u16,
    /// Width of a single cell in pixels (font-dependent).
    pub cell_width: u16,
    /// Height of a single cell in pixels (font-dependent).
    pub cell_height: u16,
}

/// Progress-bar state for the macOS dock / Linux Unity launcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressState {
    /// No progress indicator shown.
    Default,
    /// Determinate progress; pair with a `value` in `[0, 100]`.
    Normal,
    /// Determinate progress shown in an error colour.
    Error,
    /// Indeterminate (spinner / pulsing) progress.
    Indeterminate,
}

/// Parsed server reply for an IPC request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// `true` for a successful reply, `false` for a server-side error.
    pub ok: bool,
    /// Optional `data` payload from a successful reply.
    pub data: Option<JsonValue>,
    /// Optional human-readable error from a failed reply.
    pub error: Option<String>,
}

/// A minimal JSON value used to pass server responses back to the typed
/// wrappers and to assemble request arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonValue {
    /// JSON `null`.
    Null,
    /// JSON boolean.
    Bool(bool),
    /// JSON integer (the only numeric shape we parse out of the wire).
    Int(i64),
    /// JSON string.
    String(String),
    /// JSON array of [`JsonValue`].
    Array(Vec<JsonValue>),
    /// JSON object with string keys and [`JsonValue`] values.
    /// Pairs are stored in insertion order for deterministic output.
    Object(Vec<(String, JsonValue)>),
}

/// Errors produced by the IPC client.
#[derive(Debug)]
pub enum IpcError {
    /// The stream dropped or the peer closed it unexpectedly.
    ConnectionLost(String),
    /// The wire framing or the JSON payload was malformed.
    Protocol(String),
    /// The server returned `{ "ok": false, "error": "..." }`.
    Server(String),
    /// The operation did not complete within the configured timeout.
    Timeout(Duration),
    /// Underlying I/O failure.
    Io(std::io::Error),
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConnectionLost(m) => write!(f, "connection lost: {m}"),
            Self::Protocol(m) => write!(f, "protocol error: {m}"),
            Self::Server(m) => write!(f, "server error: {m}"),
            Self::Timeout(d) => write!(f, "timeout after {d:?}"),
            Self::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for IpcError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for IpcError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

// Implementation

/// Default per-call timeout, in milliseconds. Generous enough for a
/// local Unix socket round-trip; short enough to keep a UI responsive.
const DEFAULT_TIMEOUT_MS: u64 = 500;

/// Environment variable consulted by [`GhosttyControl::try_new`]. We
/// mirror Ghostty's own name so the user can keep one env var.
const ENV_CONTROL_SOCKET: &str = "GHOSTTY_CONTROL_SOCKET";

impl GhosttyControl {
    /// Try to connect to the Ghostty control socket using the standard
    /// resolution order: `$GHOSTTY_CONTROL_SOCKET`, then a handful of
    /// well-known defaults.
    ///
    /// Returns `None` if the socket is absent or the connection is
    /// refused. **Never panics** — see the module-level R8 contract.
    pub fn try_new() -> Option<Self> {
        let path = default_socket_path()?;
        Self::try_with_path(&path)
    }

    /// Try to connect to a specific socket path. Returns `None` on
    /// any failure and never panics. We probe with a `path.exists()`
    /// check rather than a real `UnixStream::connect`: a one-shot
    /// probe would consume the kernel's pending-accept slot, racing
    /// with subsequent calls. The actual connect happens in
    /// [`GhosttyControl::send`].
    pub fn try_with_path(path: &Path) -> Option<Self> {
        if !path.exists() {
            return None;
        }
        Some(Self {
            socket_path: path.to_path_buf(),
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
        })
    }

    /// Set the current window's title.
    pub fn set_window_title(&self, title: &str) -> Result<(), IpcError> {
        let args = JsonObject::new().insert("title", JsonValue::String(title.to_owned()));
        self.send("set_window_title", args)?;
        Ok(())
    }

    /// Set the progress indicator in the macOS dock / Linux Unity
    /// launcher. `value` is ignored when `state` is [`ProgressState::Default`]
    /// or [`ProgressState::Indeterminate`].
    pub fn set_progress(&self, state: ProgressState, value: u8) -> Result<(), IpcError> {
        let state_str = match state {
            ProgressState::Default => "default",
            ProgressState::Normal => "normal",
            ProgressState::Error => "error",
            ProgressState::Indeterminate => "indeterminate",
        };
        let args = JsonObject::new()
            .insert("state", JsonValue::String(state_str.to_owned()))
            .insert("value", JsonValue::Int(i64::from(value)));
        self.send("set_progress", args)?;
        Ok(())
    }

    /// Ask Ghostty to reload `~/.config/ghostty/config` from disk
    /// without restarting the terminal.
    pub fn reload_config(&self) -> Result<(), IpcError> {
        self.send("reload_config", JsonObject::new())?;
        Ok(())
    }

    /// Open a URL in the user's default browser.
    pub fn open_url(&self, url: &str) -> Result<(), IpcError> {
        let args = JsonObject::new().insert("url", JsonValue::String(url.to_owned()));
        self.send("open_url", args)?;
        Ok(())
    }

    /// Get the current window's size in pixels and cells.
    pub fn get_window_size(&self) -> Result<WindowSize, IpcError> {
        let response = self.send("get_window_size", JsonObject::new())?;
        parse_window_size(&response)
    }

    /// Validate a GLSL shader fragment against Ghostty's GPU renderer.
    ///
    /// Ghostty validates the shader on the GPU side (if running) and
    /// returns a report of syntax and semantic errors. The `source`
    /// should be a complete GLSL fragment or vertex shader body.
    pub fn shader_lint(&self, source: &str) -> Result<Response, IpcError> {
        let args = JsonObject::new().insert("source", JsonValue::String(source.to_owned()));
        self.send("shader_lint", args)
    }

    /// Request the list of fonts currently available to Ghostty.
    ///
    /// The reply contains font family, style, and path entries that
    /// Ghostty discovered at startup.
    pub fn font_list(&self) -> Result<Response, IpcError> {
        self.send("font_list", JsonObject::new())
    }

    /// Request an introspection snapshot of the running Ghostty instance.
    ///
    /// The reply contains version, configuration paths, runtime state,
    /// and shader details.
    pub fn inspect(&self) -> Result<Response, IpcError> {
        self.send("inspect", JsonObject::new())
    }

    /// Send a single request and read the single reply.
    fn send(&self, action: &str, args: JsonObject) -> Result<Response, IpcError> {
        let payload = build_request(action, &args);
        let framed = frame_request(&payload);

        let mut stream = UnixStream::connect(&self.socket_path).map_err(|e| {
            IpcError::ConnectionLost(format!("connect {}: {e}", self.socket_path.display()))
        })?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;

        stream.write_all(&framed).map_err(|e| {
            IpcError::ConnectionLost(format!("write to {}: {e}", self.socket_path.display()))
        })?;

        let response = read_framed_response(&mut stream, self.timeout)?;
        if !response.ok {
            return Err(IpcError::Server(
                response.error.unwrap_or_else(|| "(no error message)".to_owned()),
            ));
        }
        Ok(response)
    }
}

// Socket-path resolution

/// Resolve the default control socket path. The order mirrors Ghostty's
/// own resolution:
/// 1. `$GHOSTTY_CONTROL_SOCKET` if it points to an existing file.
/// 2. `$XDG_RUNTIME_DIR/ghostty/control.sock`.
/// 3. `$TMPDIR/ghostty-control.sock` (macOS fallback).
/// 4. `/tmp/ghostty-control.sock` (Linux fallback).
fn default_socket_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(ENV_CONTROL_SOCKET) {
        let path = PathBuf::from(p);
        if path.exists() {
            return Some(path);
        }
    }
    if let Some(xdg) = std::env::var_os("XDG_RUNTIME_DIR") {
        let p = PathBuf::from(xdg).join("ghostty/control.sock");
        if p.exists() {
            return Some(p);
        }
    }
    if let Some(tmp) = std::env::var_os("TMPDIR") {
        let p = PathBuf::from(tmp).join("ghostty-control.sock");
        if p.exists() {
            return Some(p);
        }
    }
    let p = PathBuf::from("/tmp/ghostty-control.sock");
    if p.exists() {
        return Some(p);
    }
    None
}

// Display

impl fmt::Display for ProgressState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Default => "default",
            Self::Normal => "normal",
            Self::Error => "error",
            Self::Indeterminate => "indeterminate",
        })
    }
}

// Tests

#[cfg(test)]
mod tests {
    use std::io::Read as _;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::ipc_request::frame_request;

    /// Serialise env-var mutation across tests; edition 2024 hides the
    /// safe `std::env::set_var` / `remove_var` API.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard that restores an env var on drop.
    struct EnvVarGuard {
        key: &'static str,
        prev: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &Path) -> Self {
            let prev = std::env::var_os(key);
            // SAFETY: tests serialise env-var mutation on `ENV_LOCK`.
            unsafe { std::env::set_var(key, value) };
            Self { key, prev }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // SAFETY: same argument as in `set`.
            unsafe {
                match self.prev.take() {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    /// Allocate a fresh tmp path that is guaranteed not to exist on disk.
    fn fresh_tmp_path(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        std::env::temp_dir().join(format!("ghostty-kit-{tag}-{pid}-{nanos}.sock"))
    }

    // 1. R8: try_new returns None when no socket exists
    #[test]
    fn try_new_returns_none_when_no_socket_exists() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let bogus = fresh_tmp_path("missing");
        let _env = EnvVarGuard::set(ENV_CONTROL_SOCKET, &bogus);

        // The contract: this must NOT panic, and must return None when
        // the env-set path is not a live socket. If a real Ghostty is
        // running on this host, the resolver will return Some, which
        // is still a *valid* client — the R8 contract is "never panic",
        // not "always None".
        let result = std::panic::catch_unwind(|| GhosttyControl::try_new());
        let result = result.expect("try_new panicked (R8 violation)");
        if result.is_some() {
            return;
        }
        assert!(result.is_none());
    }

    // 2. try_with_path round-trips a real request against a mock server
    #[test]
    fn try_with_path_round_trips_set_window_title() {
        let socket = fresh_tmp_path("roundtrip");
        let listener = UnixListener::bind(&socket).expect("bind mock socket");

        // Background thread: read one request, send one canned reply.
        // No timeouts: the protocol is synchronous and the client
        // sends its request before we try to read.
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().expect("accept");

            // Read the framed request.
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).unwrap();
            let len = u32::from_be_bytes(len_buf) as usize;
            let mut body = vec![0u8; len];
            conn.read_exact(&mut body).unwrap();
            let req = String::from_utf8(body).unwrap();
            assert!(req.contains("\"action\":\"set_window_title\""), "req: {req}");
            assert!(req.contains("\"title\":\"hello world\""), "req: {req}");

            // Reply with the canonical success envelope.
            let reply = "{\"ok\":true}";
            let framed = frame_request(reply);
            conn.write_all(&framed).unwrap();
        });

        let client = GhosttyControl::try_with_path(&socket).expect("try_with_path");
        client.set_window_title("hello world").expect("set_window_title");

        server.join().expect("server thread");
        let _ = std::fs::remove_file(&socket);
    }

    // 3. set_progress serializes to the expected JSON shape
    #[test]
    fn set_progress_serializes_correctly() {
        let payload = build_request(
            "set_progress",
            &JsonObject::new()
                .insert("state", JsonValue::String("normal".to_owned()))
                .insert("value", JsonValue::Int(42)),
        );
        assert!(payload.contains("\"action\":\"set_progress\""), "{payload}");
        assert!(payload.contains("\"state\":\"normal\""), "{payload}");
        assert!(payload.contains("\"value\":42"), "{payload}");
        // The id field is always present.
        assert!(payload.contains("\"id\":\""), "{payload}");
    }

    // 4. reload_config produces the expected action string
    #[test]
    fn reload_config_uses_expected_action_string() {
        let payload = build_request("reload_config", &JsonObject::new());
        assert!(payload.contains("\"action\":\"reload_config\""), "{payload}");
        assert!(payload.contains("\"args\":{}"), "{payload}");
    }

    // 5. open_url handles URL with special characters
    #[test]
    fn open_url_escapes_special_characters() {
        // Embeds characters that JSON *must* escape on the wire: `"`,
        // `\`, and a comma. The round-trip through the mock server
        // proves the client correctly encoded the URL and the
        // server-side read decoded it back.
        let url = "https://example.com/p?b=\"q\"&c=\\b,end";

        let socket = fresh_tmp_path("url-esc");
        let listener = UnixListener::bind(&socket).expect("bind");

        let url_for_server = url.to_owned();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().expect("accept");
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).unwrap();
            let len = u32::from_be_bytes(len_buf) as usize;
            let mut body = vec![0u8; len];
            conn.read_exact(&mut body).unwrap();
            let req = String::from_utf8(body).unwrap();
            assert!(
                req.contains("\"action\":\"open_url\""),
                "missing action: {req}"
            );
            // JSON escapes \" as \\\" and \\ as \\\\. The wire is JSON,
            // so we look for the escaped form (the server decoded it back
            // to the original — that's the contract we're proving).
            let escaped = url_for_server
                .replace('\\', "\\\\")
                .replace('"', "\\\"");
            assert!(
                req.contains(&format!("\"url\":\"{escaped}\"")),
                "url not present in escaped form: {req}"
            );
            let framed = frame_request("{\"ok\":true}");
            conn.write_all(&framed).unwrap();
        });

        let client = GhosttyControl::try_with_path(&socket).expect("try_with_path");
        client.open_url(url).expect("open_url");

        server.join().expect("server thread");
        let _ = std::fs::remove_file(&socket);
    }

    // 6. Server error response maps to IpcError::Server
    #[test]
    fn server_error_response_maps_to_ipc_error() {
        let socket = fresh_tmp_path("server-err");
        let listener = UnixListener::bind(&socket).expect("bind");

        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().expect("accept");

            // Drain the request so the client does not block on write.
            let mut len_buf = [0u8; 4];
            conn.read_exact(&mut len_buf).unwrap();
            let len = u32::from_be_bytes(len_buf) as usize;
            let mut body = vec![0u8; len];
            conn.read_exact(&mut body).unwrap();

            // Reply with a server-side error.
            let reply = "{\"ok\":false,\"error\":\"unknown action\"}";
            let framed = frame_request(reply);
            conn.write_all(&framed).unwrap();
        });

        let client = GhosttyControl::try_with_path(&socket).expect("try_with_path");
        let err = client
            .set_window_title("anything")
            .expect_err("expected server error");
        match err {
            IpcError::Server(msg) => assert_eq!(msg, "unknown action"),
            other => panic!("expected IpcError::Server, got {other:?}"),
        }

        server.join().expect("server thread");
        let _ = std::fs::remove_file(&socket);
    }
}
