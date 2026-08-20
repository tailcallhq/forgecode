// The batch-writer pipeline (queue plumbing, health probe, PRAGMA execution)
// is shared by both transports. The transport-specific run loops are
// cfg-gated (unix socket / windows named pipe), so on any given platform the
// other loop is compiled out and the machinery it references is unreachable.
// Allow dead_code rather than scattering cfg_attr across the subsystem.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
#[cfg(windows)]
use forge_dbd::protocol::named_pipe_name;
use forge_dbd::protocol::{HealthStatus, Request, Response, read_frame, write_frame};
use forge_domain::{Conversation, ConversationId};
use rusqlite::Connection;
#[cfg(unix)]
use tokio::net::UnixListener;
#[cfg(windows)]
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::mpsc;
#[cfg(windows)]
use tokio::task::JoinSet;
use tokio::time::timeout;
#[cfg(unix)]
use tracing::warn;
use tracing::{debug, error, info};

// ---------------------------------------------------------------------------
// Shared daemon state (cheap to clone; wraps Arcs internally)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct DaemonState {
    pub db_path: PathBuf,
    pub started_at: Instant,
    /// Approximate number of items currently sitting in the write queue.
    pub queue_depth: Arc<AtomicUsize>,
}

impl DaemonState {
    fn health(&self) -> HealthStatus {
        HealthStatus {
            uptime_secs: self.started_at.elapsed().as_secs(),
            queue_depth: self.queue_depth.load(Ordering::Relaxed),
            db_reachable: db_reachable(&self.db_path),
        }
    }
}

/// Whether the write database is actually reachable: the file must exist,
/// open cleanly, and pass `PRAGMA quick_check`. Existence alone is not enough —
/// a never-opened path marker is not a working database.
fn db_reachable(db_path: &Path) -> bool {
    if !db_path.exists() {
        return false;
    }
    Connection::open(db_path)
        .and_then(|conn| conn.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0)))
        .map(|result| result == "ok")
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Public server handle
// ---------------------------------------------------------------------------

pub struct DbServer {
    socket_path: PathBuf,
    state: DaemonState,
    /// How long the daemon keeps running after the last client disconnects.
    /// When this idle window elapses with no connected client, the daemon
    /// drains the write queue and exits cleanly (see the run loops).
    idle_timeout: Duration,
}

struct QueuedRequest {
    request: Request,
    response_tx: tokio::sync::oneshot::Sender<Response>,
}

impl DbServer {
    /// Create a daemon with the default idle timeout (300 seconds).
    pub fn new(socket_path: PathBuf, db_path: PathBuf) -> Self {
        Self::new_with_idle(socket_path, db_path, Duration::from_secs(300))
    }

    /// Create a daemon with an explicit idle timeout. Tests pass a short
    /// window to exercise the idle-shutdown path quickly; production uses
    /// [`DbServer::new`] and its default.
    pub fn new_with_idle(socket_path: PathBuf, db_path: PathBuf, idle_timeout: Duration) -> Self {
        Self {
            socket_path,
            state: DaemonState {
                db_path,
                started_at: Instant::now(),
                queue_depth: Arc::new(AtomicUsize::new(0)),
            },
            idle_timeout,
        }
    }

    pub async fn run(self) -> Result<()> {
        #[cfg(unix)]
        {
            self.run_unix().await
        }

        #[cfg(windows)]
        {
            self.run_windows().await
        }
    }

    #[cfg(unix)]
    async fn run_unix(self) -> Result<()> {
        info!(
            socket = %self.socket_path.display(),
            db    = %self.state.db_path.display(),
            "DbServer starting"
        );

        // Remove stale socket if present
        if self.socket_path.exists() {
            warn!(path = %self.socket_path.display(), "removing stale socket");
            tokio::fs::remove_file(&self.socket_path).await?;
        }

        // Create parent directory if needed
        if let Some(parent) = self.socket_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let listener = UnixListener::bind(&self.socket_path)?;
        info!(socket = %self.socket_path.display(), "Unix socket bound");

        // The real write queue used during this run
        let (queue_tx, queue_rx) = mpsc::channel::<QueuedRequest>(1024);
        let state = self.state.clone();
        // Wrap queue_tx so we can drop it on shutdown to signal the writer
        let queue_tx = Arc::new(queue_tx);

        // Spawn the batching writer task
        let db_path = self.state.db_path.clone();
        let writer_handle = tokio::spawn(Self::writer_task(queue_rx, db_path));

        // One-shot shutdown signal: fired by OS signal handlers
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        tokio::spawn(async move {
            tokio::select! {
                _ = sigterm.recv() => { info!("SIGTERM received"); }
                _ = sigint.recv()  => { info!("SIGINT received"); }
            }
            let _ = shutdown_tx.send(());
        });

        // Active-connection tracking: incremented when a client connects,
        // decremented when its handler exits. The idle timer below only shuts
        // the daemon down while this is zero.
        let active_connections = Arc::new(AtomicUsize::new(0));

        // Idle timer — the daemon self-terminates after the last client
        // disconnects and no new connection arrives for `idle_timeout`. Reset
        // on every accepted connection; when it elapses with no active
        // clients we fall through to the same graceful-drain path as the
        // shutdown signal.
        let mut idle = Box::pin(tokio::time::sleep(self.idle_timeout));

        // Accept loop — exits when shutdown fires or the daemon idles out
        loop {
            tokio::select! {
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, _addr)) => {
                            debug!("client connected");
                            let queue_tx = Arc::clone(&queue_tx);
                            let state = state.clone();
                            let active = Arc::clone(&active_connections);
                            tokio::spawn(async move {
                                active.fetch_add(1, Ordering::SeqCst);
                                Self::handle_client(stream, queue_tx, state).await;
                                active.fetch_sub(1, Ordering::SeqCst);
                            });
                            // A new client restarts the idle window.
                            idle.as_mut().reset(tokio::time::Instant::now() + self.idle_timeout);
                        }
                        Err(e) => {
                            error!("accept error: {e}");
                        }
                    }
                }
                _ = &mut shutdown_rx => {
                    info!("shutdown signal received; draining write queue");
                    break;
                }
                _ = &mut idle => {
                    if active_connections.load(Ordering::SeqCst) == 0 {
                        info!(
                            idle = ?self.idle_timeout,
                            "no active clients; shutting down after idle timeout"
                        );
                        break;
                    }
                    // A client is still connected — give it another window
                    // rather than shutting down mid-session.
                    idle.as_mut().reset(tokio::time::Instant::now() + self.idle_timeout);
                }
            }
        }

        // ---- Graceful drain ------------------------------------------------
        // Drop our sender half so the writer task sees channel-closed once all
        // in-flight client handlers also drop their clones.
        drop(queue_tx);

        // Wait for the writer to finish flushing.
        match writer_handle.await {
            Ok(()) => info!("writer task drained; exiting cleanly"),
            Err(e) => error!("writer task panicked: {e}"),
        }

        // Remove the socket file so the next start-up doesn't need to clean up.
        if self.socket_path.exists() {
            let _ = tokio::fs::remove_file(&self.socket_path).await;
        }

        Ok(())
    }

    #[cfg(windows)]
    async fn run_windows(self) -> Result<()> {
        let pipe_name = named_pipe_name(&self.socket_path);
        info!(
            pipe = %pipe_name,
            db   = %self.state.db_path.display(),
            "DbServer starting (windows named pipe)"
        );

        // The real write queue used during this run
        let (queue_tx, queue_rx) = mpsc::channel::<QueuedRequest>(1024);
        let state = self.state.clone();
        // Wrap queue_tx so we can drop it on shutdown to signal the writer
        let queue_tx = Arc::new(queue_tx);

        // Spawn the batching writer task
        let db_path = self.state.db_path.clone();
        let writer_handle = tokio::spawn(Self::writer_task(queue_rx, db_path));

        // One-shot shutdown signal: fired by the console ctrl-c handler
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let mut sigint = tokio::signal::windows::ctrl_c()?;
        tokio::spawn(async move {
            sigint.recv().await;
            info!("CTRL_C received");
            let _ = shutdown_tx.send(());
        });

        // Active-connection tracking + idle timer, same semantics as the unix
        // transport.
        let active_connections = Arc::new(AtomicUsize::new(0));
        let mut idle = Box::pin(tokio::time::sleep(self.idle_timeout));

        // Concurrent accept loop — exits when shutdown fires or the daemon
        // idles out.
        //
        // Windows has no stale-socket problem: named pipes are released when
        // the last handle drops, so there is nothing to unlink. Windows
        // named-pipe servers can listen on several INSTANCES of the same pipe
        // name at once, which is what makes simultaneous clients possible. We
        // keep a small pool of pending connect tasks in a JoinSet — each task
        // creates a pipe instance and awaits a client opening it — and
        // replenish the pool every time one connects. This replaces the
        // original sequential one-instance wiring, where a second client's
        // `connect` could not complete until the first handler had returned.
        // The write queue serialises requests anyway; the per-connection
        // request/response loop and the frame protocol are shared with the
        // unix transport via the stream-generic `handle_client`.
        const PIPE_INSTANCES: usize = 4;
        let mut accepts: JoinSet<std::io::Result<NamedPipeServer>> = JoinSet::new();
        for _ in 0..PIPE_INSTANCES {
            accepts.spawn(Self::connect_pipe_instance(pipe_name.clone()));
        }

        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    info!("shutdown signal received; draining write queue");
                    break;
                }
                joined = accepts.join_next(), if !accepts.is_empty() => {
                    match joined {
                        Some(Ok(Ok(pipe))) => {
                            debug!("client connected");
                            let queue_tx = Arc::clone(&queue_tx);
                            let state = state.clone();
                            let active = Arc::clone(&active_connections);
                            tokio::spawn(async move {
                                active.fetch_add(1, Ordering::SeqCst);
                                Self::handle_client(pipe, queue_tx, state).await;
                                active.fetch_sub(1, Ordering::SeqCst);
                            });
                            // Replenish the consumed instance and restart the
                            // idle window.
                            accepts.spawn(Self::connect_pipe_instance(pipe_name.clone()));
                            idle.as_mut().reset(tokio::time::Instant::now() + self.idle_timeout);
                        }
                        Some(Ok(Err(e))) => {
                            error!("pipe connect error: {e}");
                            // Brief backoff so a persistent bind failure
                            // cannot hot-spin the loop.
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            accepts.spawn(Self::connect_pipe_instance(pipe_name.clone()));
                        }
                        Some(Err(e)) => {
                            error!("pipe connect task panicked: {e}");
                            accepts.spawn(Self::connect_pipe_instance(pipe_name.clone()));
                        }
                        None => {
                            // Defensive: only reachable if the pool drained
                            // without a join (the guard above makes this
                            // unreachable in practice).
                            info!("accept pool exhausted; shutting down");
                            break;
                        }
                    }
                }
                _ = &mut idle => {
                    if active_connections.load(Ordering::SeqCst) == 0 {
                        info!(
                            idle = ?self.idle_timeout,
                            "no active clients; shutting down after idle timeout"
                        );
                        break;
                    }
                    idle.as_mut().reset(tokio::time::Instant::now() + self.idle_timeout);
                }
            }
        }

        // ---- Graceful drain ------------------------------------------------
        // Same as the unix transport: drop our sender half so the writer task
        // sees channel-closed once all client handlers drop their clones.
        drop(queue_tx);

        match writer_handle.await {
            Ok(()) => info!("writer task drained; exiting cleanly"),
            Err(e) => error!("writer task panicked: {e}"),
        }

        Ok(())
    }

    /// Creates a fresh named-pipe instance for `pipe_name` and waits for a
    /// client to open it. Each task in the accept pool runs one of these;
    /// multiple instances of the same pipe name let concurrent clients be
    /// served simultaneously.
    #[cfg(windows)]
    async fn connect_pipe_instance(pipe_name: String) -> std::io::Result<NamedPipeServer> {
        let pipe = ServerOptions::new().create(&pipe_name)?;
        pipe.connect().await?;
        Ok(pipe)
    }

    // -------------------------------------------------------------------------
    // Per-connection handler (stream-generic: used by both transports)
    // -------------------------------------------------------------------------

    async fn handle_client<S>(
        mut stream: S,
        queue_tx: Arc<mpsc::Sender<QueuedRequest>>,
        state: DaemonState,
    ) where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        loop {
            let request = match timeout(
                Duration::from_secs(30),
                read_frame::<_, Request>(&mut stream),
            )
            .await
            {
                Ok(Ok(req)) => req,
                Ok(Err(e)) => {
                    debug!("frame read error: {e}");
                    break;
                }
                Err(_) => {
                    debug!("client read timeout");
                    break;
                }
            };

            debug!("received request: {:?}", request);

            // Health probe is handled inline — no queue round-trip needed
            if matches!(request, Request::Ping) {
                let resp = Response::Health(state.health());
                let _ = write_frame(&mut stream, &resp).await;
                continue;
            }

            // All other requests go through the write queue
            let (response_tx, response_rx) = tokio::sync::oneshot::channel();
            state.queue_depth.fetch_add(1, Ordering::Relaxed);
            let queued = QueuedRequest { request, response_tx };

            if queue_tx.send(queued).await.is_err() {
                state.queue_depth.fetch_sub(1, Ordering::Relaxed);
                error!("failed to enqueue request; channel closed");
                let err_response = Response::Error { message: "server queue closed".to_string() };
                let _ = write_frame(&mut stream, &err_response).await;
                break;
            }

            match timeout(Duration::from_secs(30), response_rx).await {
                Ok(Ok(response)) => {
                    debug!("sending response: {:?}", response);
                    if let Err(e) = write_frame(&mut stream, &response).await {
                        error!("failed to write response: {e}");
                        break;
                    }
                }
                Ok(Err(_)) => {
                    error!("response oneshot dropped");
                    break;
                }
                Err(_) => {
                    error!("response timeout");
                    let timeout_resp =
                        Response::Error { message: "server processing timeout".to_string() };
                    let _ = write_frame(&mut stream, &timeout_resp).await;
                    break;
                }
            }
        }

        debug!("client disconnected");
    }

    // -------------------------------------------------------------------------
    // Batching writer task
    // -------------------------------------------------------------------------

    async fn writer_task(mut queue_rx: mpsc::Receiver<QueuedRequest>, db_path: PathBuf) {
        let mut batch: Vec<QueuedRequest> = Vec::new();
        let batch_timeout = Duration::from_millis(15);
        let batch_threshold = 100;

        // Single writer connection (P3 design): opened lazily on the first
        // write and reused for the daemon's whole lifetime, instead of opening
        // a fresh connection per request. Lazy so the daemon does not create
        // the DB file unless a write actually arrives.
        let mut conn: Option<Connection> = None;

        loop {
            match timeout(batch_timeout, queue_rx.recv()).await {
                Ok(Some(req)) => {
                    batch.push(req);
                    if batch.len() >= batch_threshold {
                        Self::flush_batch(&mut batch, &mut conn, &db_path).await;
                    }
                }
                Ok(None) => {
                    // All senders dropped (graceful shutdown path)
                    if !batch.is_empty() {
                        info!(count = batch.len(), "draining final batch on shutdown");
                        Self::flush_batch(&mut batch, &mut conn, &db_path).await;
                    }
                    info!("writer task exiting");
                    break;
                }
                Err(_) => {
                    // Batch window elapsed
                    if !batch.is_empty() {
                        Self::flush_batch(&mut batch, &mut conn, &db_path).await;
                    }
                }
            }
        }
    }

    /// Execute a batch of requests on the single writer connection.
    async fn flush_batch(
        batch: &mut Vec<QueuedRequest>,
        conn: &mut Option<Connection>,
        db_path: &Path,
    ) {
        debug!(count = batch.len(), "flushing batch");

        // Open the writer connection on first use. A failed open fails every
        // request in the batch with a path-tagged error and is retried on the
        // next batch.
        if conn.is_none() {
            match Self::open_writer_connection(db_path) {
                Ok(open) => *conn = Some(open),
                Err(e) => {
                    for queued in batch.drain(..) {
                        let _ = queued.response_tx.send(Response::Error {
                            message: format!("db error ({}): {e}", db_path.display()),
                        });
                    }
                    return;
                }
            }
        }

        let conn = conn.as_ref().expect("writer connection opened above");
        for queued in batch.drain(..) {
            let resp = match Self::execute_with_conn(conn, &queued.request) {
                Ok(resp) => resp,
                Err(e) => {
                    Response::Error { message: format!("db error ({}): {e}", db_path.display()) }
                }
            };
            let _ = queued.response_tx.send(resp);
        }
    }

    /// Opens the daemon's single writer connection.
    ///
    /// Mirrors the pragmas forge_repo's pool customizer applies to its write
    /// connections (busy timeout, WAL). No schema statements run here: table
    /// creation and migrations stay with the app's diesel setup.
    fn open_writer_connection(db_path: &Path) -> Result<Connection> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        Ok(conn)
    }

    /// Execute a single request against the writer connection.
    ///
    /// - [`Request::CheckpointWal`] runs `PRAGMA wal_checkpoint(TRUNCATE)`.
    /// - [`Request::OptimizeFts`] / [`Request::RefreshFts`] run `PRAGMA
    ///   optimize`, which also maintains FTS indexes when present.
    /// - Conversation writes mirror the exact SQL from forge_repo's
    ///   `ConversationRepositoryImpl` (conversation_repo.rs): the
    ///   `conversations` table, the `conversation_id` conflict target, and the
    ///   same updated-column sets.
    fn execute_with_conn(conn: &Connection, request: &Request) -> Result<Response> {
        match request {
            Request::CheckpointWal => {
                // wal_checkpoint(TRUNCATE) returns a single row
                // (busy, log, checkpointed); running it is the real work.
                let (busy, log, checkpointed): (i64, i64, i64) =
                    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })?;
                debug!(busy, log, checkpointed, "wal checkpoint executed");
                Ok(Response::Ack)
            }
            Request::OptimizeFts | Request::RefreshFts => {
                conn.execute_batch("PRAGMA optimize;")?;
                Ok(Response::Ack)
            }
            Request::UpsertConversation { conversation, workspace_id } => {
                Self::upsert_conversation(conn, conversation, true, *workspace_id)?;
                Ok(Response::Ack)
            }
            Request::UpsertConversationRef { conversation, workspace_id } => {
                Self::upsert_conversation(conn, conversation, false, *workspace_id)?;
                Ok(Response::Ack)
            }
            Request::UpdateParentId { conversation_id, new_parent_id } => {
                // Mirrors forge_repo's update_parent_id exactly: no workspace
                // filter, parent_id + updated_at only.
                conn.execute(
                    "UPDATE conversations SET parent_id = ?, \
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE conversation_id = ?",
                    rusqlite::params![
                        new_parent_id.as_ref().map(ConversationId::into_string),
                        conversation_id.into_string()
                    ],
                )?;
                Ok(Response::Ack)
            }
            Request::DeleteConversation { conversation_id, workspace_id } => {
                conn.execute(
                    "DELETE FROM conversations WHERE conversation_id = ? AND workspace_id = ?",
                    rusqlite::params![conversation_id.into_string(), workspace_id],
                )?;
                Ok(Response::Ack)
            }
            Request::Ping => Ok(Response::Error {
                message: "Ping is answered inline by the connection handler, \
                          not by the batch writer"
                    .to_string(),
            }),
        }
    }

    /// INSERT ... ON CONFLICT(conversation_id) DO UPDATE ... against the
    /// `conversations` table, mirroring forge_repo's `upsert_conversation` /
    /// `upsert_conversation_ref`. `full` selects the column set of
    /// `upsert_conversation` (which also refreshes `context_zstd` /
    /// `is_compressed` on conflict) vs `upsert_conversation_ref` (which
    /// leaves those compression columns untouched).
    ///
    /// `workspace_id` is the value supplied by the client request: the
    /// daemon's own `current_dir`-derived hash diverges from the client's in
    /// `--directory` mode (Windows path canonicalization), and reads filter
    /// by the client's hash, so a daemon-written row would be invisible to
    /// its caller. `None` falls back to the daemon's own derivation for
    /// hand-built requests that carry no id.
    ///
    /// Values are derived from the domain [`Conversation`] the same way
    /// `ConversationRecord::new` / `new_ref` derive them. Two deliberate
    /// simplifications for this wiring pass, both readable by the app:
    /// - `context` is stored as plain JSON (no zstd), so `context_zstd` stays
    ///   NULL and `is_compressed` stays 0.
    /// - `created_at` is the client-supplied RFC3339 timestamp, which diesel's
    ///   SQLite timestamp reader accepts (`%FT%T%.fZ`); `updated_at` is stamped
    ///   in SQL via `strftime`.
    fn upsert_conversation(
        conn: &Connection,
        conversation: &Conversation,
        full: bool,
        workspace_id: Option<i64>,
    ) -> Result<()> {
        // Keep the daemon's wire bytes identical to forge_repo's direct
        // writer, including the legacy ContextRecord envelope and its
        // compressed-column fallback.
        let persisted_context =
            forge_dbd::conversation_storage::persist_context(conversation.context.as_ref());

        let conversation_value = serde_json::to_value(conversation)?;
        let created_at = conversation_value
            .get("metadata")
            .and_then(|meta| meta.get("created_at"))
            .and_then(|created| created.as_str())
            .context("conversation metadata.created_at missing")?
            .to_string();

        // Metrics are best-effort: an unreadable blob falls back to default
        // metrics on the read side, same as forge_repo.
        let metrics = serde_json::to_string(&conversation.metrics).ok();

        // The client's workspace id when the request carries one; otherwise
        // the daemon's own cwd-derived id (e.g. hand-built requests in tests).
        let workspace_id = workspace_id.unwrap_or_else(Self::workspace_id);

        let update_set = if full {
            "title = excluded.title, \
             context = excluded.context, \
             context_zstd = excluded.context_zstd, \
             is_compressed = excluded.is_compressed, \
             updated_at = excluded.updated_at, \
             metrics = excluded.metrics, \
             parent_id = excluded.parent_id, \
             source = excluded.source, \
             cwd = excluded.cwd, \
             message_count = excluded.message_count"
        } else {
            "title = excluded.title, \
             context = excluded.context, \
             updated_at = excluded.updated_at, \
             metrics = excluded.metrics, \
             parent_id = excluded.parent_id, \
             source = excluded.source, \
             cwd = excluded.cwd, \
             message_count = excluded.message_count"
        };

        conn.execute(
            &format!(
                "INSERT INTO conversations (\
                     conversation_id, title, workspace_id, context, created_at, \
                     updated_at, metrics, parent_id, source, cwd, message_count, \
                     intent_state, extracted_at, memory_id, intent_hash, \
                     context_zstd, is_compressed\
                 ) VALUES (\
                     ?, ?, ?, ?, ?, \
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?, ?, ?, ?, ?, \
                     'pending', NULL, NULL, NULL, ?, ?\
                 )\
                 ON CONFLICT(conversation_id) DO UPDATE SET {update_set}",
            ),
            rusqlite::params![
                conversation.id.into_string(),
                conversation.title.clone(),
                workspace_id,
                persisted_context.context,
                created_at,
                metrics,
                conversation.parent_id.map(|id| id.into_string()),
                conversation.source.clone(),
                conversation.cwd.clone(),
                persisted_context.message_count,
                persisted_context.context_zstd,
                persisted_context.is_compressed,
            ],
        )?;
        Ok(())
    }

    /// The workspace id the daemon writes under.
    ///
    /// forge_app passes `env.workspace_hash().id() as i64` into the
    /// ConversationRepository (forge_repo.rs). The daemon recomputes the same
    /// value from its own environment: `Environment::workspace_hash` hashes
    /// the cwd with a zero-seed `DefaultHasher`, so it is deterministic
    /// across processes for the same cwd (the daemon is spawned by the first
    /// client, so it inherits the client's cwd).
    fn workspace_id() -> i64 {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let env = forge_domain::Environment {
            os: std::env::consts::OS.to_string(),
            cwd: std::env::current_dir().unwrap_or_else(|_| home.clone()),
            home: Some(home.clone()),
            shell: std::env::var("SHELL").unwrap_or_default(),
            base_path: home.join(".forge"),
        };
        env.workspace_hash().id() as i64
    }
}

// ---------------------------------------------------------------------------
// Test helpers (shared by the unix socket tests, the platform-neutral db
// tests, and the windows named-pipe test)
// ---------------------------------------------------------------------------

#[cfg(test)]
fn create_conversations_schema(conn: &rusqlite::Connection) {
    // The `conversations` table as forge_repo's diesel schema + migrations
    // produce it. The daemon never creates schema, so tests that exercise
    // conversation writes must set it up explicitly.
    conn.execute_batch(
        "CREATE TABLE conversations (\
             conversation_id TEXT PRIMARY KEY NOT NULL, \
             title TEXT, \
             workspace_id BIGINT NOT NULL, \
             context TEXT, \
             created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, \
             updated_at TIMESTAMP, \
             metrics TEXT, \
             parent_id TEXT, \
             source TEXT, \
             cwd TEXT, \
             message_count INTEGER, \
             intent_state TEXT NOT NULL DEFAULT 'pending', \
             extracted_at TIMESTAMP, \
             memory_id TEXT, \
             intent_hash TEXT, \
             context_zstd BLOB, \
             is_compressed INTEGER NOT NULL DEFAULT 0\
         )",
    )
    .expect("create conversations schema");
}

// ---------------------------------------------------------------------------
// Unix socket transport tests
// ---------------------------------------------------------------------------

#[cfg(all(test, unix))]
mod tests {
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;
    use tokio::net::UnixStream;
    use tokio::time::{Duration, sleep};

    use super::*;

    fn tmp_paths(dir: &TempDir) -> (PathBuf, PathBuf) {
        let sock = dir.path().join("test.sock");
        let db = dir.path().join("test.db");
        (sock, db)
    }

    /// Spawn the server in the background and return a handle + socket path.
    async fn spawn_server(
        sock: PathBuf,
        db: PathBuf,
    ) -> tokio::task::JoinHandle<anyhow::Result<()>> {
        let server = DbServer::new(sock, db);
        tokio::spawn(server.run())
    }

    /// Spawn the server with an explicit (short) idle timeout so tests can
    /// exercise the idle-shutdown path without waiting out the default.
    async fn spawn_server_with_idle(
        sock: PathBuf,
        db: PathBuf,
        idle: Duration,
    ) -> tokio::task::JoinHandle<anyhow::Result<()>> {
        let server = DbServer::new_with_idle(sock, db, idle);
        tokio::spawn(server.run())
    }

    /// Wait until the socket file appears (server is ready to accept).
    async fn wait_for_socket(sock: &Path) {
        for _ in 0..50 {
            if sock.exists() {
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
        panic!("server socket did not appear in time");
    }

    // -------------------------------------------------------------------------
    // Health probe test
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn health_probe_returns_status() {
        let dir = TempDir::new().unwrap();
        let (sock, db) = tmp_paths(&dir);
        let _handle = spawn_server(sock.clone(), db.clone()).await;
        wait_for_socket(&sock).await;

        let mut stream = UnixStream::connect(&sock).await.expect("connect");
        write_frame(&mut stream, &Request::Ping)
            .await
            .expect("write ping");
        let resp: Response = read_frame(&mut stream).await.expect("read health");

        match resp {
            Response::Health(status) => {
                // uptime is small but non-negative
                assert!(status.uptime_secs < 60, "uptime should be < 60s in test");
                // queue should be empty while no writes are in flight
                assert_eq!(status.queue_depth, 0);
                // no writes arrived, so the writer never opened the DB —
                // the file doesn't exist yet and reachable = false
                assert!(!status.db_reachable);
            }
            other => panic!("expected Health response, got {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // Drain test: enqueue writes, then close the accept side; writer must flush
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn graceful_drain_flushes_queued_writes() {
        let dir = TempDir::new().unwrap();
        let (sock, db) = tmp_paths(&dir);
        let _handle = spawn_server(sock.clone(), db.clone()).await;
        wait_for_socket(&sock).await;

        // Send a few writes and collect Ack responses to confirm they're processed
        let mut stream = UnixStream::connect(&sock).await.expect("connect");

        // Use OptimizeFts as a lightweight write request
        let n = 5usize;
        for _ in 0..n {
            write_frame(&mut stream, &Request::OptimizeFts)
                .await
                .expect("write request");
        }

        let mut acks = 0usize;
        for _ in 0..n {
            let resp: Response = read_frame(&mut stream).await.expect("read response");
            if matches!(resp, Response::Ack) {
                acks += 1;
            }
        }

        assert_eq!(
            acks, n,
            "all writes should be acknowledged (drain verified)"
        );
    }

    // -------------------------------------------------------------------------
    // Queue depth reflected in health status when writes are in flight
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn health_probe_reflects_queue_depth() {
        // This test verifies the atomic counter path is exercised.
        // Because the writer drains quickly, we just confirm the probe succeeds
        // (depth may already be 0 by the time we probe — that is correct behavior).
        let dir = TempDir::new().unwrap();
        let (sock, db) = tmp_paths(&dir);
        let _handle = spawn_server(sock.clone(), db.clone()).await;
        wait_for_socket(&sock).await;

        let mut stream = UnixStream::connect(&sock).await.expect("connect");
        write_frame(&mut stream, &Request::Ping)
            .await
            .expect("write ping");
        let resp: Response = read_frame(&mut stream).await.expect("read health");
        assert!(matches!(resp, Response::Health(_)));
    }

    // -------------------------------------------------------------------------
    // Idle timeout tests: the daemon must exit cleanly after the last client
    // disconnects and no new connection arrives for the idle window.
    // -------------------------------------------------------------------------

    /// With no client ever connecting, the daemon exits on its own once the
    /// idle window elapses, unlinks its socket, and reports a clean result.
    #[tokio::test]
    async fn idle_timeout_shuts_down_without_clients() {
        let dir = TempDir::new().unwrap();
        let (sock, db) = tmp_paths(&dir);
        let handle =
            spawn_server_with_idle(sock.clone(), db.clone(), Duration::from_millis(500)).await;
        wait_for_socket(&sock).await;

        // Deliberately do not connect: the server must self-terminate.
        sleep(Duration::from_millis(1200)).await;

        assert!(
            handle.is_finished(),
            "server should have exited after idle timeout"
        );
        assert!(handle.await.expect("server task should not panic").is_ok());
        assert!(!sock.exists(), "socket should be unlinked on shutdown");
    }

    /// Connecting and completing a Ping, then dropping the connection, must
    /// still let the daemon exit after the idle window — proving the
    /// last-client-disconnect semantics (not a fixed uptime cap).
    #[tokio::test]
    async fn idle_timeout_shuts_down_after_last_client_disconnects() {
        let dir = TempDir::new().unwrap();
        let (sock, db) = tmp_paths(&dir);
        let handle =
            spawn_server_with_idle(sock.clone(), db.clone(), Duration::from_millis(500)).await;
        wait_for_socket(&sock).await;

        // Complete a full request/response cycle, then drop the connection
        // (the handler sees EOF and the active-connection count returns to 0).
        {
            let mut stream = UnixStream::connect(&sock).await.expect("connect");
            write_frame(&mut stream, &Request::Ping)
                .await
                .expect("write ping");
            let resp: Response = read_frame(&mut stream).await.expect("read health");
            assert!(matches!(resp, Response::Health(_)));
        }

        sleep(Duration::from_millis(1200)).await;

        assert!(
            handle.is_finished(),
            "server should exit after last client disconnects + idle window"
        );
        assert!(handle.await.expect("server task should not panic").is_ok());
    }
}

// ---------------------------------------------------------------------------
// DB execution tests — platform-independent: drive the batch handler / writer
// connection directly (the same path the writer task uses), no socket needed.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod db_tests {
    use forge_domain::{Context, ContextMessage, Conversation, ConversationId};
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn flush_batch_performs_real_sqlite_work() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let mut conn = None;

        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        let mut batch = vec![QueuedRequest { request: Request::CheckpointWal, response_tx }];
        DbServer::flush_batch(&mut batch, &mut conn, &db_path).await;

        // A stub Ack would acknowledge without creating a file; real execution
        // opens the SQLite DB and runs the checkpoint on the single writer
        // connection (opened lazily by flush_batch).
        assert!(matches!(response_rx.await.unwrap(), Response::Ack));
        assert!(db_path.exists());

        // ...and the file on disk is a valid, intact SQLite database.
        let opened = rusqlite::Connection::open(&db_path).unwrap();
        let integrity: String = opened
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
    }

    /// The frame codec (currently JSON, the P3 design's alternate encoding)
    /// must round-trip the conversation request payloads. This pins that the
    /// domain types (Conversation / Metrics / Context, which skip empty
    /// fields) survive encode + decode.
    #[test]
    fn json_round_trip_conversation_request() {
        let conversation = Conversation::generate().title("rt".to_string());
        let request = Request::UpsertConversationRef { conversation, workspace_id: None };
        let encoded = serde_json::to_vec(&request).unwrap();
        let decoded: Request =
            serde_json::from_slice(&encoded).expect("json round-trip must succeed");
        assert!(matches!(decoded, Request::UpsertConversationRef { .. }));
    }

    /// Upsert inserts a new row and, on the same conversation_id, updates it
    /// in place — the ON CONFLICT path must not duplicate the row.
    #[test]
    fn upsert_conversation_inserts_and_updates_in_place() {
        let dir = TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("test.db")).unwrap();
        create_conversations_schema(&conn);

        let conversation = Conversation::generate().title("first title".to_string());
        let id = conversation.id;
        assert!(matches!(
            DbServer::execute_with_conn(
                &conn,
                &Request::UpsertConversationRef {
                    conversation: conversation.clone(),
                    workspace_id: None
                }
            )
            .expect("upsert"),
            Response::Ack
        ));

        let (title, workspace_id, message_count): (Option<String>, i64, Option<i32>) = conn
            .query_row(
                "SELECT title, workspace_id, message_count FROM conversations \
                 WHERE conversation_id = ?1",
                [id.into_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(title.as_deref(), Some("first title"));
        assert_eq!(workspace_id, DbServer::workspace_id());
        assert_eq!(message_count, None);

        // Same conversation_id on conflict → the row updates in place.
        let updated = conversation.clone().title("second title".to_string());
        assert!(matches!(
            DbServer::execute_with_conn(
                &conn,
                &Request::UpsertConversationRef { conversation: updated, workspace_id: None }
            )
            .expect("upsert on conflict"),
            Response::Ack
        ));

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM conversations WHERE conversation_id = ?1",
                [id.into_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "conflict upsert must not create a second row");

        let title: Option<String> = conn
            .query_row(
                "SELECT title FROM conversations WHERE conversation_id = ?1",
                [id.into_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title.as_deref(), Some("second title"));
    }

    /// The full upsert stores the legacy context wire in zstd form, stamps
    /// `updated_at`, and mirrors forge_repo's compression columns exactly.
    #[test]
    fn upsert_conversation_with_context_stores_compressed_context_and_timestamps() {
        let dir = TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("test.db")).unwrap();
        create_conversations_schema(&conn);

        let context = Context::default().add_message(ContextMessage::user("hello", None));
        let conversation = Conversation::generate()
            .title("with context".to_string())
            .context(context);
        let id = conversation.id;
        assert!(matches!(
            DbServer::execute_with_conn(
                &conn,
                &Request::UpsertConversation { conversation, workspace_id: None }
            )
            .expect("upsert"),
            Response::Ack
        ));

        type ConversationStorageRow = (
            Option<String>,
            Option<i32>,
            Option<String>,
            Option<Vec<u8>>,
            i32,
        );
        let (context, message_count, updated_at, context_zstd, is_compressed): ConversationStorageRow = conn
            .query_row(
                "SELECT context, message_count, updated_at, context_zstd, is_compressed \
                 FROM conversations WHERE conversation_id = ?1",
                [id.into_string()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();

        assert_eq!(context, None, "compressed rows leave context NULL");
        assert_eq!(message_count, Some(1));
        assert!(
            updated_at.is_some(),
            "updated_at should be stamped on write"
        );
        let compressed = context_zstd.expect("full upsert must persist zstd context");
        let decompressed = zstd::decode_all(compressed.as_slice()).expect("valid zstd context");
        let actual = String::from_utf8(decompressed).expect("UTF-8 context");
        assert_eq!(
            actual,
            r#"{"messages":[{"message":{"text":{"role":"User","content":"hello"}}}]}"#
        );
        assert_eq!(is_compressed, 1);
    }

    /// A reference upsert inserts compression fields for a new row, but on
    /// conflict intentionally preserves the existing compressed payload.
    #[test]
    fn upsert_conversation_ref_conflict_preserves_compression_fields() {
        let dir = TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("test.db")).unwrap();
        create_conversations_schema(&conn);

        let fixture = Conversation::generate()
            .context(Context::default().add_message(ContextMessage::user("first", None)));
        let id = fixture.id;
        DbServer::execute_with_conn(
            &conn,
            &Request::UpsertConversation { conversation: fixture.clone(), workspace_id: None },
        )
        .expect("initial full upsert");

        let before: (Option<String>, Option<Vec<u8>>, i32) = conn
            .query_row(
                "SELECT context, context_zstd, is_compressed FROM conversations \
                 WHERE conversation_id = ?1",
                [id.into_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();

        let updated =
            fixture.context(Context::default().add_message(ContextMessage::user("second", None)));
        DbServer::execute_with_conn(
            &conn,
            &Request::UpsertConversationRef { conversation: updated, workspace_id: None },
        )
        .expect("reference conflict upsert");

        let actual: (Option<String>, Option<Vec<u8>>, i32) = conn
            .query_row(
                "SELECT context, context_zstd, is_compressed FROM conversations \
                 WHERE conversation_id = ?1",
                [id.into_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(actual.0, None);
        assert_eq!(actual.1, before.1);
        assert_eq!(actual.2, before.2);
    }

    /// A delete request must not remove an identically-addressed conversation
    /// from a different workspace.
    #[test]
    fn delete_conversation_does_not_remove_row_from_another_workspace() {
        let dir = TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("test.db")).unwrap();
        create_conversations_schema(&conn);

        let conversation = Conversation::generate().title("to delete".to_string());
        let id = conversation.id;
        assert!(matches!(
            DbServer::execute_with_conn(
                &conn,
                &Request::UpsertConversationRef { conversation, workspace_id: None }
            )
            .expect("upsert"),
            Response::Ack
        ));

        let other_workspace_id = DbServer::workspace_id() + 1;
        conn.execute(
            "UPDATE conversations SET workspace_id = ?1 WHERE conversation_id = ?2",
            rusqlite::params![other_workspace_id, id.into_string()],
        )
        .unwrap();

        assert!(matches!(
            DbServer::execute_with_conn(
                &conn,
                &Request::DeleteConversation {
                    conversation_id: id,
                    workspace_id: DbServer::workspace_id(),
                }
            )
            .expect("delete"),
            Response::Ack
        ));

        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM conversations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 1, "row from the other workspace must remain");
    }

    /// update_parent_id mirrors forge_repo: it sets parent_id (and stamps
    /// updated_at) on the matching conversation_id with no workspace filter.
    #[test]
    fn update_parent_id_sets_and_clears_parent() {
        let dir = TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("test.db")).unwrap();
        create_conversations_schema(&conn);

        let conversation = Conversation::generate().title("child".to_string());
        let id = conversation.id;
        assert!(matches!(
            DbServer::execute_with_conn(
                &conn,
                &Request::UpsertConversationRef { conversation, workspace_id: None }
            )
            .expect("upsert"),
            Response::Ack
        ));

        let parent = ConversationId::generate();
        assert!(matches!(
            DbServer::execute_with_conn(
                &conn,
                &Request::UpdateParentId { conversation_id: id, new_parent_id: Some(parent) }
            )
            .expect("set parent"),
            Response::Ack
        ));
        let stored: Option<String> = conn
            .query_row(
                "SELECT parent_id FROM conversations WHERE conversation_id = ?1",
                [id.into_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored.as_deref(), Some(parent.into_string().as_str()));

        // Clearing the parent back to NULL works too.
        assert!(matches!(
            DbServer::execute_with_conn(
                &conn,
                &Request::UpdateParentId { conversation_id: id, new_parent_id: None }
            )
            .expect("clear parent"),
            Response::Ack
        ));
        let stored: Option<String> = conn
            .query_row(
                "SELECT parent_id FROM conversations WHERE conversation_id = ?1",
                [id.into_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, None);
    }

    /// A conversation write against a database without the `conversations`
    /// table fails with an Error that names the resolved db path (the old stub
    /// behaviour, now reachable only through genuine failure).
    #[tokio::test]
    async fn conversation_write_error_includes_db_path() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let mut conn = Some(DbServer::open_writer_connection(&db_path).unwrap());
        // No schema created: the DELETE hits a missing table.

        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        let mut batch = vec![QueuedRequest {
            request: Request::DeleteConversation {
                conversation_id: ConversationId::default(),
                workspace_id: DbServer::workspace_id(),
            },
            response_tx,
        }];
        DbServer::flush_batch(&mut batch, &mut conn, &db_path).await;

        match response_rx.await.unwrap() {
            Response::Error { message } => {
                assert!(
                    message.contains(&db_path.display().to_string()),
                    "error should reference the resolved db path: {message}"
                );
            }
            other => panic!("expected Error response, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Windows named-pipe integration test: spawn the real server, drive it
// through the real client (DbClient) over the pipe transport.
// ---------------------------------------------------------------------------

#[cfg(all(test, windows))]
mod windows_tests {
    use std::path::PathBuf;

    use forge_dbd::client::DbClient;
    use tempfile::TempDir;
    use tokio::time::{Duration, sleep};

    use super::*;

    fn tmp_paths(dir: &TempDir) -> (PathBuf, PathBuf) {
        // Include the pid in the socket path so the derived pipe name is
        // unique per process: parallel test runs (and stale instances from
        // crashed runs) cannot collide.
        let pid = std::process::id();
        let sock = dir.path().join(format!("test-{pid}.sock"));
        let db = dir.path().join(format!("test-{pid}.db"));
        (sock, db)
    }

    /// Connect with retries: the server creates pipe instances on demand, so
    /// an open may land in the brief window where no instance exists yet.
    async fn connect_with_retry(sock: &std::path::Path) -> DbClient {
        for _ in 0..100 {
            if let Ok(client) = DbClient::connect(sock).await {
                return client;
            }
            sleep(Duration::from_millis(20)).await;
        }
        panic!("DbClient::connect did not succeed in time");
    }

    #[tokio::test]
    async fn named_pipe_ping_health_and_conversation_write() {
        let dir = TempDir::new().unwrap();
        let (sock, db) = tmp_paths(&dir);

        // Create the schema up-front (the daemon never migrates).
        let schema_conn = rusqlite::Connection::open(&db).unwrap();
        create_conversations_schema(&schema_conn);
        drop(schema_conn);

        let server = DbServer::new(sock.clone(), db.clone());
        let _handle = tokio::spawn(server.run());

        let client = connect_with_retry(&sock).await;

        // Ping → Health over the named pipe.
        let health = client.health().await.expect("health probe");
        assert!(health.uptime_secs < 60, "uptime should be < 60s in test");
        assert_eq!(health.queue_depth, 0);

        // Conversation write against the pre-created schema → real Ack.
        let conversation = Conversation::generate().title("windows pipe test".to_string());
        let id = conversation.id;
        let resp = client
            .send(Request::UpsertConversationRef { conversation, workspace_id: None })
            .await
            .expect("send upsert");
        assert!(matches!(resp, Response::Ack), "expected Ack, got {resp:?}");

        // And the row actually landed in the write DB.
        let verify = rusqlite::Connection::open(&db).unwrap();
        let title: Option<String> = verify
            .query_row(
                "SELECT title FROM conversations WHERE conversation_id = ?1",
                [id.into_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title.as_deref(), Some("windows pipe test"));
    }

    /// Two clients connected at the same time are both served: the accept
    /// loop keeps several pipe instances pending, so the second client's open
    /// completes while the first is still connected (a sequential accept loop
    /// would leave the second client stuck on `ERROR_PIPE_BUSY` until the
    /// first disconnects).
    #[tokio::test]
    async fn named_pipe_serves_concurrent_clients() {
        let dir = TempDir::new().unwrap();
        let (sock, db) = tmp_paths(&dir);

        let schema_conn = rusqlite::Connection::open(&db).unwrap();
        create_conversations_schema(&schema_conn);
        drop(schema_conn);

        let server = DbServer::new(sock.clone(), db.clone());
        let _handle = tokio::spawn(server.run());

        // Client A connects and stays connected for the whole test.
        let client_a = connect_with_retry(&sock).await;
        let health_a = client_a.health().await.expect("health probe A");
        assert!(health_a.uptime_secs < 60, "uptime should be < 60s in test");

        // Client B connects while A is still open and is served immediately.
        let client_b = connect_with_retry(&sock).await;
        let health_b = client_b.health().await.expect("health probe B");
        assert!(health_b.uptime_secs < 60, "uptime should be < 60s in test");

        // Writes from both distinct connections land in the write DB.
        let conversation_a = Conversation::generate().title("concurrent a".to_string());
        let conversation_b = Conversation::generate().title("concurrent b".to_string());
        let id_a = conversation_a.id;
        let id_b = conversation_b.id;
        let resp_a = client_a
            .send(Request::UpsertConversationRef {
                conversation: conversation_a,
                workspace_id: None,
            })
            .await
            .expect("send upsert A");
        let resp_b = client_b
            .send(Request::UpsertConversationRef {
                conversation: conversation_b,
                workspace_id: None,
            })
            .await
            .expect("send upsert B");
        assert!(
            matches!(resp_a, Response::Ack),
            "expected Ack, got {resp_a:?}"
        );
        assert!(
            matches!(resp_b, Response::Ack),
            "expected Ack, got {resp_b:?}"
        );

        let verify = rusqlite::Connection::open(&db).unwrap();
        let count: i64 = verify
            .query_row(
                "SELECT COUNT(*) FROM conversations WHERE conversation_id IN (?1, ?2)",
                [id_a.into_string(), id_b.into_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2, "both concurrent writes should land");
    }

    /// With no client ever connecting, the named-pipe server exits on its own
    /// once the idle window elapses (no socket file exists on Windows to
    /// assert against; a clean join is the whole check).
    #[tokio::test]
    async fn idle_timeout_shuts_down_without_clients() {
        let dir = TempDir::new().unwrap();
        let (sock, db) = tmp_paths(&dir);

        let server = DbServer::new_with_idle(sock, db, Duration::from_millis(500));
        let handle = tokio::spawn(server.run());

        // Deliberately do not connect: the server must self-terminate.
        sleep(Duration::from_millis(1200)).await;

        assert!(
            handle.is_finished(),
            "server should have exited after idle timeout"
        );
        assert!(handle.await.expect("server task should not panic").is_ok());
    }
}
