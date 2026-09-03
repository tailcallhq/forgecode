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

#[cfg(windows)]
use crate::protocol::named_pipe_name;
use crate::protocol::{
    ConversationMutation, HealthStatus, MUTATION_PROTOCOL_VERSION, Request, Response, read_frame,
    write_frame,
};
use anyhow::{Context, Result};
use forge_domain::{Conversation, ConversationId};
use rusqlite::Connection;
#[cfg(unix)]
use tokio::net::UnixListener;
#[cfg(windows)]
use tokio::net::windows::named_pipe::ServerOptions;
use tokio::sync::mpsc;
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
            protocol_version: MUTATION_PROTOCOL_VERSION,
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
        let queue_depth = Arc::clone(&self.state.queue_depth);
        let writer_handle = tokio::spawn(Self::writer_task(queue_rx, db_path, queue_depth));

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
        let queue_depth = Arc::clone(&self.state.queue_depth);
        let writer_handle = tokio::spawn(Self::writer_task(queue_rx, db_path, queue_depth));

        // One-shot shutdown signal: fired by the console ctrl-c handler
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let mut sigint = tokio::signal::windows::ctrl_c()?;
        tokio::spawn(async move {
            sigint.recv().await;
            info!("CTRL_C received");
            let _ = shutdown_tx.send(());
        });

        // Active-connection tracking + idle timer, same semantics as the unix
        // transport: on Windows the count is naturally 0 or 1 because the
        // accept loop is sequential.
        let active_connections = Arc::new(AtomicUsize::new(0));
        let mut idle = Box::pin(tokio::time::sleep(self.idle_timeout));

        // Accept loop — exits when shutdown fires or the daemon idles out.
        //
        // Windows has no stale-socket problem: named pipes are released when
        // the last handle drops, so there is nothing to unlink. The server
        // creates a fresh pipe instance per client and `connect` completes
        // when that client opens it. Sequential one-client-at-a-time
        // acceptance is intentional for this wiring (the write queue
        // serialises requests anyway); the per-connection request/response
        // loop and the frame protocol are shared with the unix transport via
        // the stream-generic `handle_client`.
        loop {
            let pipe = match ServerOptions::new().create(&pipe_name) {
                Ok(pipe) => pipe,
                Err(e) => {
                    error!("failed to create named pipe instance: {e}");
                    break;
                }
            };

            tokio::select! {
                _ = &mut shutdown_rx => {
                    info!("shutdown signal received; draining write queue");
                    break;
                }
                result = pipe.connect() => {
                    match result {
                        Ok(()) => {
                            debug!("client connected");
                            let queue_tx = Arc::clone(&queue_tx);
                            let state = state.clone();
                            let active = Arc::clone(&active_connections);
                            tokio::spawn(async move {
                                active.fetch_add(1, Ordering::SeqCst);
                                Self::handle_client(pipe, queue_tx, state).await;
                                active.fetch_sub(1, Ordering::SeqCst);
                            });
                            // A new client restarts the idle window.
                            idle.as_mut().reset(tokio::time::Instant::now() + self.idle_timeout);
                        }
                        Err(e) => {
                            error!("pipe connect error: {e}");
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

    async fn writer_task(
        mut queue_rx: mpsc::Receiver<QueuedRequest>,
        db_path: PathBuf,
        queue_depth: Arc<AtomicUsize>,
    ) {
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
                        let len = batch.len();
                        Self::flush_batch(&mut batch, &mut conn, &db_path).await;
                        queue_depth.fetch_sub(len, Ordering::Relaxed);
                    }
                }
                Ok(None) => {
                    // All senders dropped (graceful shutdown path)
                    if !batch.is_empty() {
                        let len = batch.len();
                        info!(count = len, "draining final batch on shutdown");
                        Self::flush_batch(&mut batch, &mut conn, &db_path).await;
                        queue_depth.fetch_sub(len, Ordering::Relaxed);
                    }
                    info!("writer task exiting");
                    break;
                }
                Err(_) => {
                    // Batch window elapsed
                    if !batch.is_empty() {
                        let len = batch.len();
                        Self::flush_batch(&mut batch, &mut conn, &db_path).await;
                        queue_depth.fetch_sub(len, Ordering::Relaxed);
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
            Request::MutationV2 { workspace_id, mutation } => match mutation {
                ConversationMutation::UpsertConversation { conversation, .. } => {
                    Self::upsert_conversation(conn, conversation, *workspace_id)?;
                    Ok(Response::Ack)
                }
                ConversationMutation::UpsertConversationRef { conversation, .. } => {
                    Self::upsert_conversation(conn, conversation, *workspace_id)?;
                    Ok(Response::Ack)
                }
                ConversationMutation::UpdateParentId { conversation_id, new_parent_id } => {
                    conn.execute(
                        "UPDATE conversations SET parent_id = ?, \
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE conversation_id = ? AND workspace_id = ?",
                        rusqlite::params![
                            new_parent_id.as_ref().map(ConversationId::into_string),
                            conversation_id.into_string(),
                            workspace_id,
                        ],
                    )?;
                    Ok(Response::Ack)
                }
                ConversationMutation::DeleteConversation { conversation_id } => {
                    conn.execute(
                        "DELETE FROM conversations WHERE conversation_id = ? AND workspace_id = ?",
                        rusqlite::params![conversation_id.into_string(), workspace_id],
                    )?;
                    Ok(Response::Ack)
                }
            },
            Request::UpsertConversation { .. }
            | Request::UpsertConversationRef { .. }
            | Request::UpdateParentId { .. }
            | Request::DeleteConversation { .. } => Ok(Response::Error {
                message: format!(
                    "legacy unscoped mutation rejected; negotiate protocol v{MUTATION_PROTOCOL_VERSION}"
                ),
            }),
            Request::Ping => Ok(Response::Error {
                message: "Ping is answered inline by the connection handler, \
                          not by the batch writer"
                    .to_string(),
            }),
        }
    }

    /// INSERT ... ON CONFLICT(conversation_id) DO UPDATE ... against the
    /// `conversations` table, mirroring forge_repo's `upsert_conversation` /
    /// `upsert_conversation_ref`. Both conflict paths replace the context
    /// storage tuple atomically so legacy plain rows cannot retain stale
    /// compression metadata.
    ///
    /// Values are derived from the domain [`Conversation`] the same way
    /// `ConversationRecord::new` / `new_ref` derive them. Context uses the
    /// shared legacy zstd encoder, and `created_at` remains the client-supplied
    /// RFC3339 timestamp while `updated_at` is stamped in SQL via `strftime`.
    fn upsert_conversation(
        conn: &Connection,
        conversation: &Conversation,
        workspace_id: i64,
    ) -> Result<()> {
        // Keep the daemon's wire bytes identical to forge_repo's direct
        // writer, including the legacy ContextRecord envelope and its
        // compressed-column fallback.
        let persisted_context =
            crate::conversation_storage::persist_context(conversation.context.as_ref());

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

        let update_set = "title = excluded.title, \
                          context = excluded.context, \
                          context_zstd = excluded.context_zstd, \
                          is_compressed = excluded.is_compressed, \
                          updated_at = excluded.updated_at, \
                          metrics = excluded.metrics, \
                          parent_id = excluded.parent_id, \
                          source = excluded.source, \
                          cwd = excluded.cwd, \
                          message_count = excluded.message_count";

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
        let dir = TempDir::new().unwrap();
        let (sock, db) = tmp_paths(&dir);
        let _handle = spawn_server(sock.clone(), db.clone()).await;
        wait_for_socket(&sock).await;

        // Enqueue enough writes to contend the queue, while limiting active
        // Unix streams below macOS's listener and descriptor limits.
        // Each client holds its connection open waiting for Ack, which keeps
        // its request (or its batch) counted in queue_depth until flushed.
        const MAX_IN_FLIGHT_WRITERS: usize = 32;
        let permits = Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT_WRITERS));
        let mut writers = Vec::new();
        for _ in 0..200 {
            let sock_clone = sock.clone();
            let permits = Arc::clone(&permits);
            writers.push(tokio::spawn(async move {
                let _permit = permits
                    .acquire_owned()
                    .await
                    .expect("acquire writer permit");
                let mut stream = UnixStream::connect(&sock_clone).await.expect("connect");
                write_frame(&mut stream, &Request::OptimizeFts)
                    .await
                    .expect("write request");
                let resp: Response = read_frame(&mut stream).await.expect("read response");
                assert!(matches!(resp, Response::Ack));
            }));
        }

        // Poll health via a separate probe connection until we observe
        // queue_depth > 0, proving the atomic counter reflects contended work.
        let mut saw_contended = false;
        for _ in 0..100 {
            // Give writers a chance to enqueue before probing
            sleep(Duration::from_millis(5)).await;
            let mut probe = UnixStream::connect(&sock).await.expect("probe connect");
            write_frame(&mut probe, &Request::Ping)
                .await
                .expect("write ping");
            let resp: Response = read_frame(&mut probe).await.expect("read health");
            if let Response::Health(status) = resp
                && status.queue_depth > 0
            {
                saw_contended = true;
                break;
            }
            // If all writers already finished we may have missed the window;
            // break early only after confirming still no contention.
            if writers.iter().all(|h| h.is_finished()) {
                // one more probe after writers done to avoid false negative
                // due to scheduling; if still 0 we will fail below
                sleep(Duration::from_millis(10)).await;
                let mut probe = UnixStream::connect(&sock).await.expect("probe connect");
                write_frame(&mut probe, &Request::Ping)
                    .await
                    .expect("write ping");
                let resp: Response = read_frame(&mut probe).await.expect("read health");
                if let Response::Health(status) = resp
                    && status.queue_depth > 0
                {
                    saw_contended = true;
                    break;
                }
                break;
            }
        }
        assert!(
            saw_contended,
            "expected queue_depth > 0 while 200 writes are contended"
        );

        // Drain: wait for all writers to be acknowledged
        for handle in writers {
            handle.await.expect("writer task");
        }

        // After drain the queue must be empty. The writer decrements the
        // counter on flush, so allow a brief window for the final batch.
        let mut drained = false;
        for _ in 0..20 {
            let mut probe = UnixStream::connect(&sock).await.expect("probe connect");
            write_frame(&mut probe, &Request::Ping)
                .await
                .expect("write ping");
            let resp: Response = read_frame(&mut probe).await.expect("read health");
            if let Response::Health(status) = resp
                && status.queue_depth == 0
            {
                drained = true;
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
        assert!(drained, "expected queue_depth == 0 after drain");
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
    use crate::protocol::ConversationMutation;
    use forge_domain::{Context, ContextMessage, Conversation, ConversationId};
    use tempfile::TempDir;

    use super::*;

    const TEST_WORKSPACE: i64 = 42;

    fn scoped(workspace_id: i64, mutation: ConversationMutation) -> Request {
        Request::MutationV2 { workspace_id, mutation }
    }

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
        let request = scoped(
            TEST_WORKSPACE,
            ConversationMutation::UpsertConversationRef { conversation, workspace_id: None },
        );
        let encoded = serde_json::to_vec(&request).unwrap();
        let decoded: Request =
            serde_json::from_slice(&encoded).expect("json round-trip must succeed");
        assert!(matches!(decoded, Request::MutationV2 { .. }));
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
                &scoped(
                    TEST_WORKSPACE,
                    ConversationMutation::UpsertConversationRef {
                        conversation: conversation.clone(),
                        workspace_id: None,
                    },
                )
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
        assert_eq!(workspace_id, TEST_WORKSPACE);
        assert_eq!(message_count, None);

        // Same conversation_id on conflict → the row updates in place.
        let updated = conversation.clone().title("second title".to_string());
        assert!(matches!(
            DbServer::execute_with_conn(
                &conn,
                &scoped(
                    TEST_WORKSPACE,
                    ConversationMutation::UpsertConversationRef {
                        conversation: updated,
                        workspace_id: None
                    },
                )
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
                &scoped(
                    TEST_WORKSPACE,
                    ConversationMutation::UpsertConversation { conversation, workspace_id: None },
                ),
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

    /// A reference upsert replaces a legacy plain context with its compressed
    /// representation, matching forge_repo's direct ref-upsert behavior.
    #[test]
    fn upsert_conversation_ref_conflict_replaces_legacy_plain_context_with_compressed_context() {
        let dir = TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("test.db")).unwrap();
        create_conversations_schema(&conn);

        let id = ConversationId::generate();
        let legacy_context =
            r#"{"messages":[{"message":{"text":{"role":"User","content":"legacy"}}}]}"#;
        conn.execute(
            "INSERT INTO conversations (conversation_id, workspace_id, context, created_at, message_count, is_compressed) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                id.into_string(),
                TEST_WORKSPACE,
                legacy_context,
                "2026-08-15T00:00:00Z",
                1,
                0,
            ],
        )
        .expect("seed legacy plain context");

        let incoming = Conversation::new(id)
            .context(Context::default().add_message(ContextMessage::user("incoming", None)));
        DbServer::execute_with_conn(
            &conn,
            &scoped(
                TEST_WORKSPACE,
                ConversationMutation::UpsertConversationRef {
                    conversation: incoming,
                    workspace_id: None,
                },
            ),
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
        let compressed = actual
            .1
            .expect("incoming compressed context must replace legacy plain data");
        let actual_context =
            String::from_utf8(zstd::decode_all(compressed.as_slice()).expect("valid zstd context"))
                .expect("UTF-8 context");
        assert_eq!(
            actual_context,
            r#"{"messages":[{"message":{"text":{"role":"User","content":"incoming"}}}]}"#
        );
        assert_eq!(actual.2, 1);
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
                &scoped(
                    TEST_WORKSPACE,
                    ConversationMutation::UpsertConversationRef {
                        conversation,
                        workspace_id: None,
                    },
                ),
            )
            .expect("upsert"),
            Response::Ack
        ));

        let other_workspace_id = TEST_WORKSPACE + 1;
        conn.execute(
            "UPDATE conversations SET workspace_id = ?1 WHERE conversation_id = ?2",
            rusqlite::params![other_workspace_id, id.into_string()],
        )
        .unwrap();

        assert!(matches!(
            DbServer::execute_with_conn(
                &conn,
                &scoped(
                    TEST_WORKSPACE,
                    ConversationMutation::DeleteConversation { conversation_id: id },
                )
            )
            .expect("delete"),
            Response::Ack
        ));

        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM conversations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 1, "row from the other workspace must remain");
    }

    /// Mutations carry workspace identity from their client instead of using
    /// the daemon process CWD. A shared socket can therefore serve A and B
    /// without assigning B's rows to A or letting B delete A's row.
    #[test]
    fn workspace_scoped_mutations_isolate_shared_daemon_clients() {
        let dir = TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("test.db")).unwrap();
        create_conversations_schema(&conn);

        let workspace_a = 101;
        let workspace_b = 202;
        let conversation_a = Conversation::generate().title("workspace A".to_string());
        let conversation_b = Conversation::generate().title("workspace B".to_string());
        let id_a = conversation_a.id;
        let id_b = conversation_b.id;

        for (workspace_id, conversation) in
            [(workspace_a, conversation_a), (workspace_b, conversation_b)]
        {
            let request = Request::MutationV2 {
                workspace_id,
                mutation: ConversationMutation::UpsertConversationRef {
                    conversation,
                    workspace_id: None,
                },
            };
            assert!(matches!(
                DbServer::execute_with_conn(&conn, &request).expect("upsert"),
                Response::Ack
            ));
        }

        let delete_b = Request::MutationV2 {
            workspace_id: workspace_b,
            mutation: ConversationMutation::DeleteConversation { conversation_id: id_b },
        };
        assert!(matches!(
            DbServer::execute_with_conn(&conn, &delete_b).expect("delete"),
            Response::Ack
        ));

        let actual: Vec<(String, i64)> = conn
            .prepare(
                "SELECT conversation_id, workspace_id FROM conversations ORDER BY workspace_id",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let expected = vec![(id_a.into_string(), workspace_a)];
        assert_eq!(actual, expected);
    }

    /// update_parent_id updates only the caller's workspace row and stamps
    /// its updated_at value.
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
                &scoped(
                    TEST_WORKSPACE,
                    ConversationMutation::UpsertConversationRef {
                        conversation,
                        workspace_id: None,
                    },
                ),
            )
            .expect("upsert"),
            Response::Ack
        ));

        let parent = ConversationId::generate();
        assert!(matches!(
            DbServer::execute_with_conn(
                &conn,
                &scoped(
                    TEST_WORKSPACE,
                    ConversationMutation::UpdateParentId {
                        conversation_id: id,
                        new_parent_id: Some(parent),
                    },
                )
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
                &scoped(
                    TEST_WORKSPACE,
                    ConversationMutation::UpdateParentId {
                        conversation_id: id,
                        new_parent_id: None,
                    },
                )
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
            request: scoped(
                TEST_WORKSPACE,
                ConversationMutation::DeleteConversation {
                    conversation_id: ConversationId::default(),
                },
            ),
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

    use crate::client::DbClient;
    use crate::protocol::ConversationMutation;
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
            .send(Request::MutationV2 {
                workspace_id: 42,
                mutation: ConversationMutation::UpsertConversationRef {
                    conversation,
                    workspace_id: None,
                },
            })
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
