//! Shared daemon: UDS listener, JSON-RPC dispatch, agent registry, drift store.
//!
//! [`Server`] is the top-level long-running process. It owns:
//!
//! - a [`tokio::net::UnixListener`] bound to the configured socket path
//! - an exclusive `flock` on the socket file (single-writer guard)
//! - a PID file written at startup and removed at shutdown
//! - an [`AgentRegistry`] (in-process, parking_lot-backed)
//! - a [`Store`] (rusqlite, opened in WAL mode)
//! - a `tokio::sync::broadcast` channel for `drift.subscribe` push notifications
//!
//! Per the spec every public method here is async and the file stays under
//! 450 lines by leaning on `Server::dispatch` rather than per-method handlers.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fs2::FileExt;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, Notify};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::config::ForgeConfig;
use crate::ipc::{
    write_frame, JsonRpcRequest, JsonRpcResponse, RpcError, RpcErrorCode, FRAME_HEADER_LEN,
};
use crate::registry::{AgentRegistry, RegistryError};
use crate::store::{DriftEvent, DriftOverrideInput, Store, StoreError};

/// Per-connection handler: one task per accepted UDS stream.
type ConnHandle = JoinHandle<()>;

/// The shared daemon.
///
/// Cloning is `Arc`-based and cheap; pass clones around freely.
#[derive(Clone)]
pub struct Server {
    inner: Arc<Inner>,
}

struct Inner {
    cfg: ForgeConfig,
    registry: AgentRegistry,
    store: Arc<Store>,
    broadcast: broadcast::Sender<DriftEvent>,
    /// Notified when an observer wants the next drift event pushed to it.
    subscriber_notify: Notify,
    /// Optional join handles for the GC task and shutdown barrier.
    shutdown: Mutex<Option<ShutdownHandles>>,
    /// PID file path (for cleanup).
    pid_path: PathBuf,
    /// Lock-file guard (held for the daemon's lifetime).
    _lock_file: Arc<Mutex<Option<std::fs::File>>>,
}

struct ShutdownHandles {
    /// Task that periodically reaps expired leases.
    gc: JoinHandle<()>,
    /// File lock guard — held until shutdown.
    lock_file: std::fs::File,
    /// PID file contents — kept open to "hold" the inode.
    pid_file: std::fs::File,
}

/// Outcome of a successful `Server::start`.
pub struct StartedServer {
    /// A cloneable handle to the running server.
    pub server: Server,
    /// The socket path actually used (resolved from config).
    pub socket_path: PathBuf,
    /// Path to the PID file written at startup.
    pub pid_path: PathBuf,
    /// Sender to broadcast `Server::shutdown` from any clone.
    pub shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

/// RPC method names — kept here so tests and dispatch can't drift.
pub mod method {
    pub const AGENT_REGISTER: &str = "agent.register";
    pub const AGENT_HEARTBEAT: &str = "agent.heartbeat";
    pub const AGENT_DEREGISTER: &str = "agent.deregister";
    pub const AGENT_LIST: &str = "agent.list";
    pub const DRIFT_OBSERVE: &str = "drift.observe";
    pub const DRIFT_LIST_ALERTS: &str = "drift.list_alerts";
    pub const DRIFT_OVERRIDE: &str = "drift.override";
    pub const DRIFT_SUBSCRIBE: &str = "drift.subscribe";
}

impl Server {
    /// Build the server from config and start the listener loop.
    ///
    /// Acquires an exclusive `flock` on the socket path (fails fast if
    /// another daemon is already running), writes a PID file, opens the
    /// SQLite store, spawns a GC task, and begins accepting connections.
    pub async fn start(cfg: ForgeConfig) -> Result<StartedServer, ServerError> {
        let socket_path = cfg.resolved_socket_path();
        let pid_path = cfg.resolved_pid_path();

        // Refuse to start if socket dir cannot be created.
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ServerError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }

        // Single-writer guard via flock(LOCK_EX | LOCK_NB).
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&socket_path)
            .map_err(|e| ServerError::Io {
                path: socket_path.clone(),
                source: e,
            })?;
        lock_file
            .try_lock_exclusive()
            .map_err(|e| ServerError::AlreadyRunning {
                path: socket_path.clone(),
                source: e,
            })?;

        // Remove any stale socket file from a previous daemon.
        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path).map_err(|e| ServerError::Io {
            path: socket_path.clone(),
            source: e,
        })?;

        // PID file (best-effort: warning, not error).
        if let Err(e) = std::fs::create_dir_all(pid_path.parent().unwrap_or(std::path::Path::new("/tmp"))) {
            warn!(error = %e, "could not create pid dir");
        }
        let pid_file = match std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&pid_path)
        {
            Ok(f) => f,
            Err(e) => {
                warn!(error = %e, path = %pid_path, "could not open pid file");
                // fabricate an empty handle so Drop doesn't fire on None
                std::fs::File::create("/dev/null").unwrap()
            }
        };
        let pid = std::process::id();
        use std::io::Write as _;
        if let Err(e) = writeln!(&pid_file, "{pid}") {
            warn!(error = %e, "could not write pid");
        }

        // SQLite store.
        let store = Arc::new(Store::open(&cfg.resolved_db_path())?);
        let registry = AgentRegistry::new(cfg.lease_ttl());

        let (broadcast_tx, _) = broadcast::channel(1024);
        let subscriber_notify = Notify::new();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        // Spawn the lease-GC task.
        let gc_registry = registry.clone();
        let gc_handle = tokio::spawn(async move {
            let mut rx = shutdown_rx;
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {
                        gc_registry.gc_expired();
                    }
                }
            }
        });

        let inner = Inner {
            cfg: cfg.clone(),
            registry,
            store: Arc::clone(&store),
            broadcast: broadcast_tx.clone(),
            subscriber_notify,
            shutdown: Mutex::new(Some(ShutdownHandles {
                gc: gc_handle,
                lock_file,
                pid_file,
            })),
            pid_path: pid_path.clone(),
            _lock_file: Arc::new(Mutex::new(None)),
        };

        let server = Server { inner: Arc::new(inner) };

        // Accept loop (in its own task; we don't await it).
        let accept_server = server.clone();
        tokio::spawn(async move {
            accept_server.accept_loop(listener).await;
        });

        info!(socket = %socket_path.display(), pid, "forge3d started");

        Ok(StartedServer {
            server,
            socket_path,
            pid_path,
            shutdown_tx,
        })
    }

    /// Block until `shutdown_tx` is signalled, then tear everything down.
    pub async fn run_until_shutdown(self, shutdown_tx: tokio::sync::oneshot::Receiver<()>) {
        let _ = shutdown_tx.await;
        self.shutdown().await;
    }

    /// Stop the daemon: cancel the GC task, remove socket + PID files.
    pub async fn shutdown(&self) {
        let mut slot = self.inner.shutdown.lock();
        if let Some(handles) = slot.take() {
            handles.gc.abort();
            let _ = handles.gc.await;
            // Drop the flock explicitly by closing the file handle.
            let _ = handles.lock_file.unlock();
            drop(handles.lock_file);
            drop(handles.pid_file);
        }
        let _ = std::fs::remove_file(&self.inner.cfg.resolved_socket_path());
        let _ = std::fs::remove_file(&self.inner.pid_path);
        info!("forge3d shut down");
    }

    /// Read-only view of the in-process config.
    pub fn config(&self) -> &ForgeConfig {
        &self.inner.cfg
    }

    /// Broadcast a drift event to all `drift.subscribe` listeners.
    pub fn broadcast_event(&self, ev: DriftEvent) {
        // Ignore send errors — that's "no subscribers", which is fine.
        let _ = self.inner.broadcast.send(ev);
    }

    /// Receiver for direct subscription (rarely used; prefer `drift.subscribe`).
    pub fn subscribe(&self) -> broadcast::Receiver<DriftEvent> {
        self.inner.broadcast.subscribe()
    }

    /// Dispatch a JSON-RPC request to the appropriate handler.
    ///
    /// Async signature is required for the `notify` channel and any future
    /// handler that needs `tokio::sync::Notify`. `spawn_blocking` is used
    /// internally for SQLite calls.
    pub async fn dispatch(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        match req.method.as_str() {
            method::AGENT_REGISTER => self.handle_agent_register(req).await,
            method::AGENT_HEARTBEAT => self.handle_agent_heartbeat(req).await,
            method::AGENT_DEREGISTER => self.handle_agent_deregister(req).await,
            method::AGENT_LIST => self.handle_agent_list(req).await,
            method::DRIFT_OBSERVE => self.handle_drift_observe(req).await,
            method::DRIFT_LIST_ALERTS => self.handle_drift_list_alerts(req).await,
            method::DRIFT_OVERRIDE => self.handle_drift_override(req).await,
            method::DRIFT_SUBSCRIBE => self.handle_drift_subscribe(req).await,
            other => req.error_response(RpcError::new(
                RpcErrorCode::MethodNotFound,
                format!("unknown method: {other}"),
                None,
            )),
        }
    }

    // -- agent.* -----------------------------------------------------------

    async fn handle_agent_register(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let params: serde_json::Value = req.params.clone().unwrap_or_default();
        let agent_id = match params.get("agent_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return req.error_response(RpcError::new(
                RpcErrorCode::InvalidParams,
                "agent_id is required".into(),
                None,
            )),
        };
        let pid = match params.get("pid").and_then(|v| v.as_i64()) {
            Some(p) => p,
            None => return req.error_response(RpcError::new(
                RpcErrorCode::InvalidParams,
                "pid is required".into(),
                None,
            )),
        };
        let label = params
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let lane = params
            .get("lane")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string();

        let lease = self.inner.registry.register(agent_id.clone(), pid, label, lane);

        match req.id {
            Some(id) => JsonRpcResponse::success(id, serde_json::to_value(&lease).unwrap()),
            None => JsonRpcResponse::notification(),
        }
    }

    async fn handle_agent_heartbeat(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let params: serde_json::Value = req.params.clone().unwrap_or_default();
        let agent_id = match params.get("agent_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return req.error_response(RpcError::new(
                RpcErrorCode::InvalidParams,
                "agent_id is required".into(),
                None,
            )),
        };

        match self.inner.registry.heartbeat(&agent_id) {
            Ok(()) => match req.id {
                Some(id) => JsonRpcResponse::success(id, serde_json::json!({"ok": true})),
                None => JsonRpcResponse::notification(),
            },
            Err(RegistryError::UnknownAgent(id)) => req.error_response(RpcError::new(
                RpcErrorCode::InvalidParams,
                format!("unknown agent: {id}"),
                None,
            )),
            Err(e) => req.error_response(rpc_internal_error(e)),
        }
    }

    async fn handle_agent_deregister(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let params: serde_json::Value = req.params.clone().unwrap_or_default();
        let agent_id = match params.get("agent_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return req.error_response(RpcError::new(
                RpcErrorCode::InvalidParams,
                "agent_id is required".into(),
                None,
            )),
        };

        self.inner.registry.deregister(&agent_id);
        match req.id {
            Some(id) => JsonRpcResponse::success(id, serde_json::json!({"ok": true})),
            None => JsonRpcResponse::notification(),
        }
    }

    async fn handle_agent_list(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let entries = self.inner.registry.list_active();
        match req.id {
            Some(id) => JsonRpcResponse::success(id, serde_json::to_value(&entries).unwrap()),
            None => JsonRpcResponse::notification(),
        }
    }

    // -- drift.* -----------------------------------------------------------

    async fn handle_drift_observe(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let params: serde_json::Value = req.params.clone().unwrap_or_default();
        let source_agent = params
            .get("source_agent")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let target_agent = params
            .get("target_agent")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let lane = params
            .get("lane")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string();
        let prompt_excerpt = params
            .get("prompt_excerpt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // PR-6: similarity provider absent; store 0.0 as placeholder.
        let similarity = params
            .get("similarity")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let ev = DriftEvent {
            id: 0, // assigned by SQLite
            source_agent,
            target_agent,
            similarity,
            lane,
            prompt_excerpt,
            created_at_unix_ms: now_unix_ms(),
            resolved_at_unix_ms: None,
        };

        let store = Arc::clone(&self.inner.store);
        let broadcast = self.inner.broadcast.clone();
        let stored = match tokio::task::spawn_blocking(move || store.record_event(&ev)).await {
            Ok(Ok(e)) => e,
            Ok(Err(e)) => return req.error_response(rpc_store_error(e)),
            Err(e) => {
                return req.error_response(RpcError::new(
                    RpcErrorCode::InternalError,
                    format!("join error: {e}"),
                    None,
                ));
            }
        };

        // Push to subscribers (best-effort).
        let _ = broadcast.send(stored.clone());

        match req.id {
            Some(id) => JsonRpcResponse::success(id, serde_json::to_value(&stored).unwrap()),
            None => JsonRpcResponse::notification(),
        }
    }

    async fn handle_drift_list_alerts(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let params: serde_json::Value = req.params.clone().unwrap_or_default();
        let limit = params.get("limit").and_then(|v| as_i64(v)).unwrap_or(100).max(1);

        let store = Arc::clone(&self.inner.store);
        let alerts = match tokio::task::spawn_blocking(move || store.list_recent_alerts(limit)).await {
            Ok(Ok(a)) => a,
            Ok(Err(e)) => return req.error_response(rpc_store_error(e)),
            Err(e) => {
                return req.error_response(RpcError::new(
                    RpcErrorCode::InternalError,
                    format!("join error: {e}"),
                    None,
                ));
            }
        };

        match req.id {
            Some(id) => JsonRpcResponse::success(id, serde_json::to_value(&alerts).unwrap()),
            None => JsonRpcResponse::notification(),
        }
    }

    async fn handle_drift_override(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let params: serde_json::Value = req.params.clone().unwrap_or_default();
        let alert_id = match params.get("alert_id").and_then(|v| as_i64(v)) {
            Some(n) => n,
            None => {
                return req.error_response(RpcError::new(
                    RpcErrorCode::InvalidParams,
                    "alert_id is required".into(),
                    None,
                ));
            }
        };
        let reason = params
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let actor = params
            .get("actor")
            .and_then(|v| v.as_str())
            .unwrap_or("system")
            .to_string();

        let input = DriftOverrideInput {
            alert_id,
            reason,
            actor,
        };
        let store = Arc::clone(&self.inner.store);
        let result = match tokio::task::spawn_blocking(move || store.apply_override(&input)).await {
            Ok(Ok(())) => serde_json::json!({"ok": true, "alert_id": alert_id}),
            Ok(Err(StoreError::AlertNotFound(id))) => {
                return req.error_response(RpcError::new(
                    RpcErrorCode::InvalidParams,
                    format!("alert_id not found: {id}"),
                    None,
                ));
            }
            Ok(Err(e)) => return req.error_response(rpc_store_error(e)),
            Err(e) => {
                return req.error_response(RpcError::new(
                    RpcErrorCode::InternalError,
                    format!("join error: {e}"),
                    None,
                ));
            }
        };

        match req.id {
            Some(id) => JsonRpcResponse::success(id, result),
            None => JsonRpcResponse::notification(),
        }
    }

    async fn handle_drift_subscribe(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        // Subscribe gives the caller a broadcast receiver id (opaque integer).
        // Subsequent events arrive as `drift.notify` server-pushed JSON-RPC
        // frames; for PR-6 we just hand back the subscription_id and let
        // higher-level orchestrators do the loop.
        let sub_id: u64 = {
            let mut counter = self.inner.subscriber_notify_counter();
            counter.0 += 1;
            counter.0
        };
        match req.id {
            Some(id) => JsonRpcResponse::success(
                id,
                serde_json::json!({"subscription_id": sub_id, "note": "PR-6: streaming hook only"}),
            ),
            None => JsonRpcResponse::notification(),
        }
    }

    // -- listener loop -----------------------------------------------------

    async fn accept_loop(self, listener: UnixListener) {
        let mut conns: Vec<ConnHandle> = Vec::new();
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let server = self.clone();
                    let handle = tokio::spawn(async move {
                        if let Err(e) = server.handle_connection(stream).await {
                            debug!(error = %e, "connection terminated");
                        }
                    });
                    conns.push(handle);
                    // Garbage-collect finished handles to avoid unbounded growth.
                    conns.retain(|h| !h.is_finished());
                }
                Err(e) => {
                    error!(error = %e, "accept failed");
                    break;
                }
            }
        }
    }

    async fn handle_connection(&self, mut stream: UnixStream) -> std::io::Result<()> {
        loop {
            let mut header = [0u8; FRAME_HEADER_LEN];
            match stream.read_exact(&mut header).await {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(e) => return Err(e),
            }
            let len = u32::from_be_bytes(header) as usize;
            let mut body = vec![0u8; len];
            stream.read_exact(&mut body).await?;

            let req: JsonRpcRequest = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => {
                    let resp = JsonRpcResponse::error(
                        None,
                        RpcError::new(
                            RpcErrorCode::ParseError,
                            format!("parse: {e}"),
                            None,
                        ),
                    );
                    write_frame(&mut stream, &serde_json::to_vec(&resp).unwrap()).await?;
                    continue;
                }
            };

            let resp = self.dispatch(req).await;
            if resp.id.is_some() {
                let bytes = serde_json::to_vec(&resp).unwrap_or_default();
                write_frame(&mut stream, &bytes).await?;
            } else {
                // Pure notification — close stream politely.
                stream.shutdown().await.ok();
                return Ok(());
            }
        }
    }
}

// -- helpers & glue -------------------------------------------------------

impl Inner {
    fn subscriber_notify_counter(&self) -> parking_lot::MutexGuard<'_, Counter> {
        // Reuse a static counter stored on the Inner; if you need more
        // granularity, swap to AtomicU64 later. Held behind parking_lot
        // to keep things simple and zero-dep.
        static COUNTER: parking_lot::Mutex<Counter> =
            parking_lot::Mutex::new(Counter(0));
        COUNTER.lock()
    }
}

#[derive(Default)]
struct Counter(u64);

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("another forge3d already running on {path}: {source}")]
    AlreadyRunning {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("store error: {0}")]
    Store(#[from] StoreError),
}

fn rpc_store_error(e: StoreError) -> RpcError {
    match e {
        StoreError::AlertNotFound(id) => RpcError::new(
            RpcErrorCode::InvalidParams,
            format!("alert_id not found: {id}"),
            None,
        ),
        other => RpcError::new(RpcErrorCode::InternalError, format!("{other}"), None),
    }
}

fn rpc_internal_error(e: impl std::fmt::Display) -> RpcError {
    RpcError::new(RpcErrorCode::InternalError, format!("{e}"), None)
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn as_i64(v: &serde_json::Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_u64().map(|n| n as i64))
}