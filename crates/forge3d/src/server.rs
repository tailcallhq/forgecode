//! forge3d daemon server — agent registry + drift dispatch over UDS.
//!
//! # Structure
//!
//! | Concern              | Module / type             |
//! |----------------------|---------------------------|
//! | Clock abstraction    | [`Clock`] / [`system_clock`] / [`fixed_clock`] |
//! | Socket path helpers  | [`Sockets`]               |
//! | Daemon orchestration | [`Server`] (builder)      |
//! | Connection handler   | `handle_connection`       |
//!
//! # Example
//!
//! ```ignore
//! use std::path::Path;
//! use std::sync::Arc;
//! use tokio_util::sync::CancellationToken;
//!
//! let shutdown = CancellationToken::new();
//! let server = Arc::new(Server::new().with_clock(system_clock()));
//! server.serve(Path::new("/tmp/forge3d.sock"), shutdown).await?;
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::net::UnixListener;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::error::{Forge3Error, Result};
use crate::pidfile::PidFile;
use crate::protocol::{self, ErrorBody, ErrorResponse, Request, Response, SuccessResponse};
use crate::registry::Registry;

use forge_drift::{AlertId, DriftDetector, OverrideReason};

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

/// Returns the current time in milliseconds since the Unix epoch.
///
/// The concrete type is `Arc<dyn Fn() -> i64 + Send + Sync>` so callers can
/// substitute a fixed clock for deterministic testing.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Real wall clock that reads `SystemTime::now()` on every invocation.
pub fn system_clock() -> Clock {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    })
}

/// Clock that always returns the same value (useful in tests).
pub fn fixed_clock(now: i64) -> Clock {
    Arc::new(move || now)
}

// ---------------------------------------------------------------------------
// Sockets
// ---------------------------------------------------------------------------

/// Helper that computes the UDS socket path under a given base directory.
///
/// The socket is always named `forge3d.sock`.
#[derive(Debug, Clone)]
pub struct Sockets {
    /// Directory containing the socket (and usually the pidfile + logs).
    pub socket_dir: PathBuf,
    /// Full path to the UDS socket file.
    pub socket_path: PathBuf,
}

impl Sockets {
    pub fn new(base_dir: &Path) -> Self {
        let socket_dir = base_dir.to_path_buf();
        let socket_path = base_dir.join("forge3d.sock");
        Self {
            socket_dir,
            socket_path,
        }
    }
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// The forge3d daemon — holds an agent [`Registry`], an optional
/// [`PidFile`] (for exclusive-daemon guarantees), an optional
/// [`DriftDetector`], and a [`Clock`] for time.
///
/// Build via the fluent builder, then wrap in `Arc` and call `serve`:
///
/// ```ignore
/// let server = Arc::new(
///     Server::new()
///         .with_pidfile(pidfile)
///         .with_drift_detector(detector)
///         .with_clock(my_clock),
/// );
/// server.serve(&socket_path, shutdown).await?;
/// ```
pub struct Server {
    registry: Registry,
    pidfile: Option<PidFile>,
    drift_detector: Option<DriftDetector>,
    clock: Clock,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("registry", &self.registry)
            .field("pidfile", &self.pidfile)
            .finish_non_exhaustive()
    }
}

impl Server {
    /// Create a new server with default (system) clock and no pidfile /
    /// drift detector. Configure extras with the builder methods below.
    pub fn new() -> Self {
        Self {
            registry: Registry::new(),
            pidfile: None,
            drift_detector: None,
            clock: system_clock(),
        }
    }

    /// Attach a pidfile guard — the handle is held for the lifetime of the
    /// server (its `Drop` releases the `flock`).
    pub fn with_pidfile(mut self, pidfile: PidFile) -> Self {
        self.pidfile = Some(pidfile);
        self
    }

    /// Attach a drift detector so `drift.observe` / `drift.override`
    /// methods become available.
    pub fn with_drift_detector(mut self, detector: DriftDetector) -> Self {
        self.drift_detector = Some(detector);
        self
    }

    /// Override the clock (used by tests to avoid wall-clock dependency).
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    // -- JSON-RPC dispatch ---------------------------------------------------

    /// Dispatch a parsed JSON-RPC request and produce a response.
    ///
    /// Supported methods:
    ///
    /// | Method              | Params (JSON)                                                     |
    /// |---------------------|-------------------------------------------------------------------|
    /// | `agent.register`    | `{ agent_id, pid, lane?, prompt_excerpt? }`                      |
    /// | `agent.heartbeat`   | `{ agent_id }`                                                    |
    /// | `agent.deregister`  | `{ agent_id }`                                                    |
    /// | `agent.list`        | `{ now_unix_ms? }`                                                |
    /// | `drift.observe`     | `{ agent_id, prompt, lane? }`                                     |
    /// | `drift.override`    | `{ alert_id, reason }`                                            |
    pub async fn dispatch(&self, req: &Request) -> Response {
        let id = req.id.clone().unwrap_or(serde_json::Value::Null);

        let out = match req.method.as_str() {
            "agent.register" => self.handle_register(&req.params),
            "agent.heartbeat" => self.handle_heartbeat(&req.params),
            "agent.deregister" => self.handle_deregister(&req.params),
            "agent.list" => self.handle_list(&req.params),
            "drift.observe" => self.handle_drift_observe(&req.params).await,
            "drift.override" => self.handle_drift_override(&req.params),
            unknown => Err(Forge3Error::Protocol(format!("unknown method: {unknown}"))),
        };

        match out {
            Ok(value) => Response::Success(SuccessResponse {
                jsonrpc: "2.0".into(),
                result: value,
                id,
            }),
            Err(e) => {
                let (code, message) = match &e {
                    Forge3Error::Protocol(msg) => (-32600, msg.clone()),
                    Forge3Error::UnknownAgent(a) => {
                        (-32010, format!("unknown agent: {a}"))
                    }
                    Forge3Error::UnknownAlert(a) => {
                        (-32011, format!("unknown alert: {a}"))
                    }
                    _ => (-32603, e.to_string()),
                };
                Response::Error(ErrorResponse {
                    jsonrpc: "2.0".into(),
                    error: ErrorBody { code, message },
                    id,
                })
            }
        }
    }

    // -- UDS serve loop ------------------------------------------------------

    /// Bind to `socket_path` and accept incoming frame-based connections
    /// until `shutdown` is cancelled.
    ///
    /// # Task-lifecycle convention (P2.4)
    ///
    /// - The accept loop `select!`s on the `shutdown` token so it exits cleanly
    ///   without waiting for the next client.
    /// - Each per-connection task is tracked in a [`JoinSet`]; on shutdown the
    ///   set is aborted and awaited so no orphaned tasks remain.
    ///
    /// **Note**: `self` must be wrapped in an `Arc` because `tokio::spawn`
    /// requires `'static` lifetimes.
    pub async fn serve(self: &Arc<Self>, socket_path: &Path, shutdown: CancellationToken) -> Result<()> {
        // Remove any stale socket file from a previous run.
        if socket_path.exists() {
            std::fs::remove_file(socket_path)?;
        }

        let listener = UnixListener::bind(socket_path)?;
        info!("forge3d listening on {}", socket_path.display());

        // Track all per-connection tasks so we can await/abort them on exit.
        let mut tasks: JoinSet<()> = JoinSet::new();

        loop {
            tokio::select! {
                // Clean shutdown: cancel all in-flight connection tasks.
                _ = shutdown.cancelled() => {
                    info!("forge3d shutting down; aborting {} in-flight connection(s)", tasks.len());
                    tasks.abort_all();
                    while tasks.join_next().await.is_some() {}
                    return Ok(());
                }

                accept_result = listener.accept() => {
                    let (stream, _addr) = match accept_result {
                        Ok(pair) => pair,
                        Err(e) => {
                            error!("accept error: {e}");
                            return Err(e.into());
                        }
                    };

                    let server = self.clone();
                    tasks.spawn(async move {
                        if let Err(e) = handle_connection(&server, stream).await {
                            warn!("connection error: {e}");
                        }
                    });
                }
            }

            // Reap any tasks that have already finished to keep the set bounded.
            while let Some(_result) = tasks.try_join_next() {}
        }
    }

    // ------------------------------------------------------------------
    // Internal handler helpers
    // ------------------------------------------------------------------

    fn handle_register(
        &self,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, Forge3Error> {
        let agent_id = params["agent_id"]
            .as_str()
            .ok_or_else(|| Forge3Error::Protocol("missing agent_id".into()))?;
        let pid = params["pid"]
            .as_u64()
            .ok_or_else(|| Forge3Error::Protocol("missing or invalid pid".into()))?
            as u32;
        let lane = params["lane"].as_str().unwrap_or("building");
        let prompt_excerpt = params["prompt_excerpt"].as_str();
        let now = (self.clock)();

        let info = self
            .registry
            .upsert(agent_id, pid, lane, prompt_excerpt, now);
        Ok(serde_json::json!({ "agent": info }))
    }

    fn handle_heartbeat(
        &self,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, Forge3Error> {
        let agent_id = params["agent_id"]
            .as_str()
            .ok_or_else(|| Forge3Error::Protocol("missing agent_id".into()))?;
        let now = (self.clock)();

        match self.registry.heartbeat(agent_id, now) {
            Some(info) => Ok(serde_json::json!({ "agent": info })),
            None => Err(Forge3Error::UnknownAgent(agent_id.to_string())),
        }
    }

    fn handle_deregister(
        &self,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, Forge3Error> {
        let agent_id = params["agent_id"]
            .as_str()
            .ok_or_else(|| Forge3Error::Protocol("missing agent_id".into()))?;
        let removed = self.registry.deregister(agent_id);
        Ok(serde_json::json!({ "removed": removed }))
    }

    fn handle_list(
        &self,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, Forge3Error> {
        let now = params
            .get("now_unix_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| (self.clock)());
        let agents = self.registry.list_active(now);
        Ok(serde_json::json!({ "agents": agents }))
    }

    async fn handle_drift_observe(
        &self,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, Forge3Error> {
        let detector = self.drift_detector.as_ref().ok_or_else(|| {
            Forge3Error::Protocol("drift detection not configured".into())
        })?;
        let agent_id = params["agent_id"]
            .as_str()
            .ok_or_else(|| Forge3Error::Protocol("missing agent_id".into()))?;
        let prompt = params["prompt"]
            .as_str()
            .ok_or_else(|| Forge3Error::Protocol("missing prompt".into()))?;
        let lane = params["lane"].as_str().unwrap_or("building");
        let now = (self.clock)();

        let event = detector.observe(agent_id, prompt, lane, now).await;
        Ok(serde_json::json!({ "event": event }))
    }

    fn handle_drift_override(
        &self,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, Forge3Error> {
        let detector = self.drift_detector.as_ref().ok_or_else(|| {
            Forge3Error::Protocol("drift detection not configured".into())
        })?;
        let alert_id: AlertId = serde_json::from_value(params["alert_id"].clone())
            .map_err(|_| Forge3Error::Protocol("missing or invalid alert_id".into()))?;
        let reason: OverrideReason = serde_json::from_value(params["reason"].clone())
            .map_err(|_| Forge3Error::Protocol("missing or invalid reason".into()))?;

        detector.override_alert(alert_id, reason);
        Ok(serde_json::json!({ "success": true }))
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Connection handler
// ---------------------------------------------------------------------------

/// Handle a single UDS connection — loop reading frames, dispatching, and
/// writing responses.
///
/// Notifications (requests with no `id`) are silently consumed per the
/// JSON-RPC 2.0 spec.
async fn handle_connection(server: &Server, stream: tokio::net::UnixStream) -> Result<()> {
    let (mut reader, mut writer) = tokio::io::split(stream);

    loop {
        let bytes = match protocol::decode_frame(&mut reader).await {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Ok(()), // clean EOF
            Err(e) => {
                warn!("decode_frame error: {e}");
                return Err(e);
            }
        };

        let req = match protocol::parse_request(&bytes) {
            Ok(r) => r,
            Err(e) => {
                warn!("parse_request error: {e}");
                return Err(e);
            }
        };

        // Notifications (no `id`) MUST NOT receive a response.
        if req.id.is_none() {
            continue;
        }

        let resp = server.dispatch(&req).await;
        let payload = serde_json::to_vec(&resp.to_json())?;
        protocol::write_frame(&mut writer, &payload).await?;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ------------------------------------------------------------------
    // Clock tests
    // ------------------------------------------------------------------

    #[test]
    fn system_clock_returns_positive() {
        let clock = system_clock();
        let t = clock();
        assert!(t > 1_700_000_000_000, "epoch millis should be reasonable");
    }

    #[test]
    fn fixed_clock_returns_exact_value() {
        let clock = fixed_clock(42);
        assert_eq!(clock(), 42);
        assert_eq!(clock(), 42); // idempotent
    }

    // ------------------------------------------------------------------
    // Sockets tests
    // ------------------------------------------------------------------

    #[test]
    fn sockets_computes_paths() {
        let sk = Sockets::new(Path::new("/tmp/forge3d"));
        assert_eq!(sk.socket_dir, Path::new("/tmp/forge3d"));
        assert_eq!(sk.socket_path, Path::new("/tmp/forge3d/forge3d.sock"));
    }

    // ------------------------------------------------------------------
    // Dispatch tests
    // ------------------------------------------------------------------

    fn test_server() -> Server {
        Server::new().with_clock(fixed_clock(1000))
    }

    fn mk_req(method: &str, params: serde_json::Value, id: u64) -> Request {
        Request {
            jsonrpc: "2.0".into(),
            method: method.into(),
            id: Some(serde_json::Value::Number(id.into())),
            params,
        }
    }

    fn assert_success(resp: &Response) -> serde_json::Value {
        match resp {
            Response::Success(s) => s.result.clone(),
            Response::Error(e) => panic!("expected success, got error: {:?}", e),
        }
    }

    fn assert_error(resp: &Response, expected_code: i32) {
        match resp {
            Response::Success(s) => panic!("expected error, got success: {:?}", s),
            Response::Error(e) => assert_eq!(e.error.code, expected_code),
        }
    }

    #[tokio::test]
    async fn dispatch_register_and_heartbeat() {
        let srv = test_server();

        // Register
        let req = mk_req("agent.register", json!({"agent_id": "alice", "pid": 100, "lane": "building"}), 1);
        let resp = srv.dispatch(&req).await;
        let val = assert_success(&resp);
        assert_eq!(val["agent"]["agent_id"], "alice");
        assert_eq!(val["agent"]["pid"], 100);

        // Heartbeat
        let req = mk_req("agent.heartbeat", json!({"agent_id": "alice"}), 2);
        let resp = srv.dispatch(&req).await;
        let val = assert_success(&resp);
        assert_eq!(val["agent"]["agent_id"], "alice");

        // Heartbeat unknown agent
        let req = mk_req("agent.heartbeat", json!({"agent_id": "unknown"}), 3);
        let resp = srv.dispatch(&req).await;
        assert_error(&resp, -32010);
    }

    #[tokio::test]
    async fn dispatch_register_and_deregister() {
        let srv = test_server();
        let req = mk_req("agent.register", json!({"agent_id": "bob", "pid": 200, "lane": "exploring"}), 1);
        srv.dispatch(&req).await;

        let req = mk_req("agent.deregister", json!({"agent_id": "bob"}), 2);
        let resp = srv.dispatch(&req).await;
        let val = assert_success(&resp);
        assert_eq!(val["removed"], true);

        // Second deregister — removed is false
        let req = mk_req("agent.deregister", json!({"agent_id": "bob"}), 3);
        let resp = srv.dispatch(&req).await;
        let val = assert_success(&resp);
        assert_eq!(val["removed"], false);
    }

    #[tokio::test]
    async fn dispatch_list() {
        let srv = test_server();
        srv.dispatch(
            &mk_req("agent.register", json!({"agent_id": "a", "pid": 1, "lane": "building"}), 1),
        )
        .await;
        srv.dispatch(
            &mk_req("agent.register", json!({"agent_id": "b", "pid": 2, "lane": "shipped"}), 2),
        )
        .await;

        // List at a time where both should be alive (now = 1000, lease = 60s)
        let req = mk_req("agent.list", json!({"now_unix_ms": 1000}), 3);
        let resp = srv.dispatch(&req).await;
        let val = assert_success(&resp);
        let agents = val["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 2);
    }

    #[tokio::test]
    async fn dispatch_unknown_method() {
        let srv = test_server();
        let req = mk_req("unknown.method", json!({}), 1);
        let resp = srv.dispatch(&req).await;
        assert_error(&resp, -32600);
    }

    #[tokio::test]
    async fn dispatch_register_missing_agent_id() {
        let srv = test_server();
        let req = mk_req("agent.register", json!({"pid": 1}), 1);
        let resp = srv.dispatch(&req).await;
        assert_error(&resp, -32600);
    }

    #[tokio::test]
    async fn dispatch_drift_not_configured() {
        let srv = test_server(); // no drift detector
        let req = mk_req("drift.observe", json!({"agent_id": "a", "prompt": "hello"}), 1);
        let resp = srv.dispatch(&req).await;
        assert_error(&resp, -32600);
    }

    // ------------------------------------------------------------------
    // Serve cancellation test (P2.4)
    // ------------------------------------------------------------------

    /// Verify that `serve` exits cleanly when the `CancellationToken` is
    /// triggered, without waiting for a new connection to arrive.
    #[tokio::test]
    async fn serve_exits_on_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("test.sock");

        let shutdown = CancellationToken::new();
        let server = Arc::new(Server::new().with_clock(fixed_clock(0)));

        // Spawn serve in a task; cancel it immediately after it has bound the socket.
        let srv = server.clone();
        let sock_path = socket.clone();
        let token = shutdown.clone();
        let serve_task = tokio::spawn(async move {
            srv.serve(&sock_path, token).await
        });

        // Give the task time to bind, then cancel.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        shutdown.cancel();

        // The task should return Ok(()) promptly.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            serve_task,
        )
        .await
        .expect("serve did not exit within 2s after cancellation")
        .expect("task panicked");

        assert!(result.is_ok(), "serve returned an error: {:?}", result);
    }
}
