use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use forge_dbd::client::DbClient;
use forge_dbd::protocol::{Request, Response};
use forge_domain::{
    Conversation, ConversationId, ConversationRepository, ConversationSummary, ForgeExportOptions,
    ForgeExportReport, ForgeForgetOptions, ForgeForgetReport, ForgeImportOptions,
    ForgeImportReport, ForgeMigrateReport, HeliosdoctorDbStats, MigrateOptions,
};
use tracing::{debug, info, warn};

use crate::conversation::ConversationRepositoryImpl;

/// Set once per process: the first routed write that fails to connect
/// attempts to spawn the daemon. A later connect failure in the same process
/// never respawns — either the daemon came up (and subsequent connects
/// succeed) or it did not, and re-spawning would just repeat the wait.
static SPAWN_ATTEMPTED: AtomicBool = AtomicBool::new(false);

/// Decorator that routes the hot-path conversation WRITES through the
/// `forge-dbd` single-writer daemon while keeping reads on the direct diesel
/// path (P3 design: writes serialise in the daemon, reads stay latency
/// sensitive and local).
///
/// Only four methods are routed to the daemon — `upsert_conversation`,
/// `upsert_conversation_ref`, `update_parent_id`, `delete_conversation` — the
/// exact set the daemon protocol implements. Every other
/// [`ConversationRepository`] method is a plain pass-through to `inner`.
///
/// ## Sync-over-async mechanics
///
/// The original plan called for a lazily-built current-thread tokio Runtime
/// with `block_on`, on the assumption that the trait methods were SYNC. That
/// assumption does not hold: `ConversationRepository` is an `async_trait`, so
/// these methods are async and are invoked from tokio worker contexts
/// (`ForgeConversationService` and friends await the trait directly). The
/// daemon round-trip is therefore a plain `.await` on `DbClient::send` — no
/// dedicated Runtime, no `Mutex`, no `block_on`-reentrancy hazard, and no
/// `Handle::current()` risk. We deliberately do NOT spawn a separate runtime
/// or wrap the trait in sync form, which would only add complexity.
///
/// ## Client lifecycle
///
/// `DbClient` is created lazily on the first routed write and cached in a
/// `tokio::sync::OnceCell`. A failed connect leaves the cell empty, so the
/// next write retries (best-effort reconnect). `DbClient::send` opens a fresh
/// connection per request anyway (client.rs), so caching only skips the
/// connect probe.
///
/// ## Daemon spawn
///
/// When the first routed write cannot connect, the client spawns the daemon
/// (binary from `FORGE_DBD_BIN` or `forge_dbd` on PATH) exactly once per
/// process, then polls the socket for ~2 s before falling back to the direct
/// path. A later connect failure in the same process skips the spawn and
/// falls back immediately — the daemon either came up or it did not.
///
/// ## Fallback semantics
///
/// A direct fallback is safe only before a daemon request can have been sent:
/// failed initial connect or failed daemon spawn. Once [`DbClient::send`]
/// starts its fresh request stream, a transport error, daemon error response,
/// or unexpected response is indeterminate: the daemon may have committed the
/// write. Those outcomes are surfaced to the caller and are never replayed
/// through `inner`.
pub struct DaemonConversationRepository {
    inner: Arc<ConversationRepositoryImpl>,
    socket_path: PathBuf,
    client: tokio::sync::OnceCell<DbClient>,
    /// Daemon binary to spawn on the first routed write, resolved from
    /// `FORGE_DBD_BIN` at construction (see
    /// [`forge_domain::Environment::dbd_bin_path`]). `None` means "look up
    /// `forge_dbd` on PATH" at spawn time.
    dbd_bin: Option<PathBuf>,
    /// The client's workspace id (hash of its cwd), stamped onto daemon-side
    /// upserts. The daemon derives its own id from ITS current directory,
    /// which diverges from the client's in `--directory` mode (Windows path
    /// canonicalization). Reads filter by the client's hash, so daemon-written
    /// rows must carry it or they would be invisible to the caller.
    workspace_id: i64,
}

/// Result of attempting a daemon-routed conversation write.
///
/// `Unavailable` means no daemon request was sent, so the direct repository
/// may safely execute the write. `Indeterminate` means the daemon had the
/// opportunity to observe the request; direct fallback would risk a duplicate
/// write and must be skipped.
enum DaemonWriteOutcome {
    Ack,
    Unavailable,
    Indeterminate(anyhow::Error),
}

impl DaemonConversationRepository {
    pub fn new(
        inner: Arc<ConversationRepositoryImpl>,
        socket_path: PathBuf,
        workspace_id: i64,
    ) -> Self {
        Self {
            inner,
            socket_path,
            client: tokio::sync::OnceCell::new(),
            dbd_bin: default_dbd_bin(),
            workspace_id,
        }
    }

    /// Test-only constructor: injects the daemon binary path so tests can
    /// point at a guaranteed-missing binary without mutating process env.
    #[cfg(test)]
    fn new_with_bin(
        inner: Arc<ConversationRepositoryImpl>,
        socket_path: PathBuf,
        dbd_bin: Option<PathBuf>,
        workspace_id: i64,
    ) -> Self {
        Self {
            inner,
            socket_path,
            client: tokio::sync::OnceCell::new(),
            dbd_bin,
            workspace_id,
        }
    }

    /// Attempt one daemon round-trip for a write request.
    ///
    /// Only an initial connection/spawn failure is classified as
    /// [`DaemonWriteOutcome::Unavailable`]. All failures after
    /// [`DbClient::send`] begins are indeterminate because the daemon may have
    /// recorded the request before the client lost its response.
    async fn try_daemon(&self, request: Request) -> DaemonWriteOutcome {
        let client = match self
            .client
            .get_or_try_init(|| DbClient::connect(&self.socket_path))
            .await
        {
            Ok(client) => client,
            Err(err) => {
                debug!(
                    socket = %self.socket_path.display(),
                    error = %err,
                    "forge_dbd unavailable; attempting to spawn daemon"
                );
                match self.spawn_and_connect().await {
                    Some(client) => client,
                    None => {
                        debug!(
                            socket = %self.socket_path.display(),
                            "forge_dbd still unavailable; falling back to direct write"
                        );
                        return DaemonWriteOutcome::Unavailable;
                    }
                }
            }
        };
        match client.send(request).await {
            Ok(Response::Ack) => DaemonWriteOutcome::Ack,
            Ok(Response::Error { message }) => {
                warn!(
                    message = %message,
                    "forge_dbd rejected write; not replaying an indeterminate write"
                );
                DaemonWriteOutcome::Indeterminate(anyhow::anyhow!(
                    "forge_dbd rejected write after receiving the request: {message}"
                ))
            }
            Ok(other) => {
                warn!(
                    response = ?other,
                    "forge_dbd returned an unexpected response; not replaying an indeterminate write"
                );
                DaemonWriteOutcome::Indeterminate(anyhow::anyhow!(
                    "forge_dbd returned an unexpected response after receiving the request: {other:?}"
                ))
            }
            Err(err) => {
                warn!(
                    error = %err,
                    "forge_dbd request transport failed; not replaying an indeterminate write"
                );
                DaemonWriteOutcome::Indeterminate(err.context(
                    "forge_dbd request outcome is indeterminate; direct fallback skipped to avoid duplicate write",
                ))
            }
        }
    }

    /// Apply the safety policy shared by every daemon-routed write.
    ///
    /// The direct fallback runs only when no daemon request was sent.
    async fn write_or_fallback<F, Fut>(&self, request: Request, fallback: F) -> anyhow::Result<()>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<()>>,
    {
        match self.try_daemon(request).await {
            DaemonWriteOutcome::Ack => Ok(()),
            DaemonWriteOutcome::Unavailable => fallback().await,
            DaemonWriteOutcome::Indeterminate(error) => Err(error),
        }
    }

    /// Attempts to bring the daemon up and connect to it.
    ///
    /// Spawns the daemon exactly once per process (guarded by
    /// [`SPAWN_ATTEMPTED`]); a second connect failure in the same process
    /// skips the spawn and falls straight back to the direct write. After a
    /// successful spawn the socket is polled for ~2 s — the daemon needs a
    /// moment to bind (the named-pipe `ERROR_PIPE_BUSY` retry inside
    /// `DbClient::connect` also helps) — and the connected client is cached
    /// in the [`OnceCell`](tokio::sync::OnceCell) for subsequent writes.
    async fn spawn_and_connect(&self) -> Option<&DbClient> {
        if SPAWN_ATTEMPTED.swap(true, Ordering::SeqCst) {
            return None;
        }
        if !self.spawn_daemon() {
            // Spawn failed (e.g. binary not found): no point polling a socket
            // nothing will bind. The guard is already set, so we do not retry.
            return None;
        }
        for _ in 0..10 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if let Ok(client) = DbClient::connect(&self.socket_path).await {
                let _ = self.client.set(client);
                return self.client.get();
            }
        }
        None
    }

    /// Spawns the `forge-dbd` daemon as a detached child process.
    ///
    /// Returns `false` (after logging) when the binary cannot be spawned,
    /// e.g. `FORGE_DBD_BIN` points at a missing file.
    ///
    /// The child inherits `FORGE_WRITE_DB_PATH` when the parent has it set,
    /// so the daemon writes to the same database as the client, and
    /// `FORGE_DBD_SOCKET` when set, so a daemon bound to a custom socket path
    /// is reachable at the same place the client expects it (both sides honor
    /// the variable; the default `~/.forge/.forge.db.sock` is used when unset).
    fn spawn_daemon(&self) -> bool {
        let bin = self
            .dbd_bin
            .clone()
            .unwrap_or_else(|| PathBuf::from("forge_dbd"));
        let mut cmd = std::process::Command::new(&bin);
        if let Ok(write_db) = std::env::var("FORGE_WRITE_DB_PATH") {
            cmd.env("FORGE_WRITE_DB_PATH", write_db);
        }
        if let Ok(socket) = std::env::var("FORGE_DBD_SOCKET") {
            cmd.env("FORGE_DBD_SOCKET", socket);
        }
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        match cmd.spawn() {
            Ok(_) => {
                info!(
                    bin = %bin.display(),
                    "spawned forge-dbd daemon; waiting for it to accept connections"
                );
                true
            }
            Err(e) => {
                warn!(
                    bin = %bin.display(),
                    error = %e,
                    "failed to spawn forge-dbd daemon; falling back to direct write"
                );
                false
            }
        }
    }
}

/// Resolves the daemon binary path from the environment, mirroring
/// [`forge_domain::Environment::dbd_bin_path`]: `FORGE_DBD_BIN` if set,
/// otherwise `None` (spawn-time `forge_dbd` PATH lookup). The method ignores
/// `self` — it only reads the environment — so a throwaway
/// [`forge_domain::Environment`] suffices.
fn default_dbd_bin() -> Option<PathBuf> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let env = forge_domain::Environment {
        os: std::env::consts::OS.to_string(),
        cwd: std::env::current_dir().unwrap_or_else(|_| home.clone()),
        home: Some(home),
        shell: std::env::var("SHELL").unwrap_or_default(),
        base_path: PathBuf::from("."),
    };
    env.dbd_bin_path()
}

#[async_trait::async_trait]
impl ConversationRepository for DaemonConversationRepository {
    // -----------------------------------------------------------------------
    // Daemon-routed writes
    // -----------------------------------------------------------------------

    async fn upsert_conversation(&self, conversation: Conversation) -> anyhow::Result<()> {
        self.write_or_fallback(
            Request::UpsertConversation {
                conversation: conversation.clone(),
                workspace_id: Some(self.workspace_id),
            },
            || self.inner.upsert_conversation(conversation),
        )
        .await
    }

    async fn upsert_conversation_ref(&self, conversation: &Conversation) -> anyhow::Result<()> {
        self.write_or_fallback(
            Request::UpsertConversationRef {
                conversation: conversation.clone(),
                workspace_id: Some(self.workspace_id),
            },
            || self.inner.upsert_conversation_ref(conversation),
        )
        .await
    }

    async fn update_parent_id(
        &self,
        conversation_id: &ConversationId,
        new_parent_id: Option<&ConversationId>,
    ) -> anyhow::Result<()> {
        self.write_or_fallback(
            Request::UpdateParentId {
                conversation_id: *conversation_id,
                new_parent_id: new_parent_id.cloned(),
            },
            || self.inner.update_parent_id(conversation_id, new_parent_id),
        )
        .await
    }

    async fn delete_conversation(&self, conversation_id: &ConversationId) -> anyhow::Result<()> {
        self.write_or_fallback(
            Request::DeleteConversation {
                conversation_id: *conversation_id,
                workspace_id: self.inner.workspace_id(),
            },
            || self.inner.delete_conversation(conversation_id),
        )
        .await
    }

    // -----------------------------------------------------------------------
    // Pass-throughs (reads + maintenance stay direct)
    // -----------------------------------------------------------------------

    async fn get_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> anyhow::Result<Option<Conversation>> {
        self.inner.get_conversation(conversation_id).await
    }

    async fn get_all_conversations(
        &self,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        self.inner.get_all_conversations(limit).await
    }

    async fn get_last_conversation(&self) -> anyhow::Result<Option<Conversation>> {
        self.inner.get_last_conversation().await
    }

    async fn get_conversations_by_parent(
        &self,
        parent_id: &ConversationId,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        self.inner.get_conversations_by_parent(parent_id).await
    }

    async fn get_parent_conversations(
        &self,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        self.inner.get_parent_conversations(limit).await
    }

    async fn get_parent_conversations_lite(
        &self,
        limit: Option<usize>,
        all_workspaces: bool,
    ) -> anyhow::Result<Option<Vec<ConversationSummary>>> {
        self.inner
            .get_parent_conversations_lite(limit, all_workspaces)
            .await
    }

    async fn get_conversations_by_source(
        &self,
        source: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        self.inner.get_conversations_by_source(source, limit).await
    }

    async fn search_conversations(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<Conversation>> {
        self.inner.search_conversations(query, limit).await
    }

    async fn get_conversation_snippet(
        &self,
        conversation_id: &ConversationId,
        query: &str,
        token_count: usize,
    ) -> anyhow::Result<Option<String>> {
        self.inner
            .get_conversation_snippet(conversation_id, query, token_count)
            .await
    }

    async fn optimize_fts_index(&self) -> anyhow::Result<()> {
        self.inner.optimize_fts_index().await
    }

    async fn refresh_fts_index(&self) -> anyhow::Result<()> {
        self.inner.refresh_fts_index().await
    }

    async fn get_conversations_by_cwd(
        &self,
        cwd: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        self.inner.get_conversations_by_cwd(cwd, limit).await
    }

    async fn mark_intent_state(
        &self,
        conversation_id: &ConversationId,
        new_state: &str,
    ) -> anyhow::Result<()> {
        self.inner
            .mark_intent_state(conversation_id, new_state)
            .await
    }

    async fn list_prune_eligible(
        &self,
        workspace_id: Option<i64>,
        limit: usize,
    ) -> anyhow::Result<Vec<Conversation>> {
        self.inner.list_prune_eligible(workspace_id, limit).await
    }

    async fn prune_conversation(&self, conversation_id: &ConversationId) -> anyhow::Result<()> {
        self.inner.prune_conversation(conversation_id).await
    }

    async fn rewind_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> anyhow::Result<Option<Conversation>> {
        self.inner.rewind_conversation(conversation_id).await
    }

    async fn compress_uncompressed_contexts(&self) -> anyhow::Result<(usize, usize, usize)> {
        self.inner.compress_uncompressed_contexts().await
    }

    async fn import_forge_db(&self, source: PathBuf) -> anyhow::Result<ForgeImportReport> {
        self.inner.import_forge_db(source).await
    }

    async fn import_forge_db_with_options(
        &self,
        source: PathBuf,
        options: &ForgeImportOptions,
    ) -> anyhow::Result<ForgeImportReport> {
        self.inner
            .import_forge_db_with_options(source, options)
            .await
    }

    async fn export_forge_db(
        &self,
        dest: PathBuf,
        options: &ForgeExportOptions,
    ) -> anyhow::Result<ForgeExportReport> {
        self.inner.export_forge_db(dest, options).await
    }

    async fn database_stats(&self) -> anyhow::Result<HeliosdoctorDbStats> {
        self.inner.database_stats().await
    }

    async fn forget_conversations(
        &self,
        options: &ForgeForgetOptions,
    ) -> anyhow::Result<ForgeForgetReport> {
        self.inner.forget_conversations(options).await
    }

    async fn migrate_data_dir(
        &self,
        options: &MigrateOptions,
    ) -> anyhow::Result<ForgeMigrateReport> {
        self.inner.migrate_data_dir(options).await
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    #[cfg(unix)]
    use forge_dbd::protocol::read_frame;
    use forge_domain::{Conversation, ConversationId, WorkspaceHash};
    use pretty_assertions::assert_eq;
    #[cfg(unix)]
    use tokio::net::UnixListener;
    #[cfg(unix)]
    use tokio::sync::oneshot;

    use super::*;
    use crate::conversation::ConversationRepositoryImpl;
    use crate::database::DatabasePool;

    /// Serializes the decorator tests that touch the process-wide spawn guard
    /// ([`SPAWN_ATTEMPTED`]) so they cannot race each other.
    static SPAWN_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn in_memory_inner() -> Arc<ConversationRepositoryImpl> {
        let pool = Arc::new(DatabasePool::in_memory().expect("in-memory pool"));
        Arc::new(ConversationRepositoryImpl::new(pool, WorkspaceHash::new(0)))
    }

    /// The decorator must fall back to the direct implementation when the
    /// daemon is absent: a socket path that does not exist makes
    /// `DbClient::connect` fail, and the write still lands in the inner
    /// repository.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // The process-wide spawn state makes these tests serial by design.
    async fn falls_back_to_direct_write_when_daemon_absent() -> anyhow::Result<()> {
        let _serial = SPAWN_GUARD.lock().unwrap();
        // Mark the spawn as already attempted so this test never launches a
        // real daemon; the assertion is about the fallback, not the spawn.
        SPAWN_ATTEMPTED.store(true, Ordering::SeqCst);

        let inner = in_memory_inner();
        let repo = DaemonConversationRepository::new(
            inner.clone(),
            PathBuf::from("/nonexistent/.forge.db.sock"),
            0,
        );

        let conversation = Conversation::new(ConversationId::generate())
            .title(Some("daemon-fallback".to_string()));
        let id = conversation.id;

        repo.upsert_conversation(conversation).await?;

        let actual = inner.get_conversation(&id).await?;
        assert_eq!(
            actual.expect("row persisted via direct fallback").title,
            Some("daemon-fallback".to_string())
        );
        Ok(())
    }

    /// With `FORGE_DBD_BIN` pointing at a binary that does not exist, the
    /// first routed write attempts to spawn it exactly once, fails, and falls
    /// back to the direct path — the row still persists. A second write does
    /// not re-attempt the spawn (the guard is set) and falls back without
    /// hanging.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // The process-wide spawn state makes these tests serial by design.
    async fn spawn_is_attempted_once_then_falls_back() -> anyhow::Result<()> {
        let _serial = SPAWN_GUARD.lock().unwrap();
        // Reset the process-wide guard so this test exercises the spawn path
        // deterministically.
        SPAWN_ATTEMPTED.store(false, Ordering::SeqCst);

        let inner = in_memory_inner();
        let repo = DaemonConversationRepository::new_with_bin(
            inner.clone(),
            PathBuf::from("/nonexistent/.forge.db.sock"),
            Some(PathBuf::from("/definitely/missing/forge_dbd_bin")),
            0,
        );

        let conversation =
            Conversation::new(ConversationId::generate()).title(Some("spawn-fallback".to_string()));
        let id = conversation.id;

        repo.upsert_conversation(conversation).await?;
        let actual = inner.get_conversation(&id).await?;
        assert_eq!(
            actual.expect("row persisted via direct fallback").title,
            Some("spawn-fallback".to_string())
        );

        // Second write: the spawn guard is set, so no respawn (and no 2 s
        // retry wait) — straight to the direct fallback.
        let second =
            Conversation::new(ConversationId::generate()).title(Some("second".to_string()));
        let second_id = second.id;
        repo.upsert_conversation(second).await?;
        let actual = inner.get_conversation(&second_id).await?;
        assert_eq!(
            actual
                .expect("second row persisted via direct fallback")
                .title,
            Some("second".to_string())
        );

        Ok(())
    }

    /// Once a daemon accepts a write request, a lost ACK is indeterminate: it
    /// may have committed the write, so the decorator must return an explicit
    /// error instead of replaying the request through the direct repository.
    #[cfg(unix)]
    #[tokio::test]
    async fn does_not_fallback_after_daemon_records_request_then_loses_ack() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let socket_path = temp_dir.path().join("forge-dbd.sock");
        let listener = UnixListener::bind(&socket_path)?;
        let (recorded_tx, recorded_rx) = oneshot::channel();

        tokio::spawn(async move {
            // DbClient::connect probes first; accepting and closing this
            // connection makes the next one the actual write transport.
            let (probe, _) = listener.accept().await.unwrap();
            drop(probe);

            let (mut write_stream, _) = listener.accept().await.unwrap();
            let request: Request = read_frame(&mut write_stream).await.unwrap();
            recorded_tx.send(request).unwrap();
            // Simulate a daemon that committed/recorded the request, then
            // crashes before emitting Ack.
        });

        let inner = in_memory_inner();
        let repo = DaemonConversationRepository::new(inner.clone(), socket_path, 0);
        let conversation =
            Conversation::new(ConversationId::generate()).title(Some("ack-loss".to_string()));
        let id = conversation.id;

        let actual = repo.upsert_conversation(conversation.clone()).await;
        assert!(actual.is_err());

        let actual = recorded_rx.await.expect("daemon recorded request");
        let expected = Request::UpsertConversation { conversation, workspace_id: Some(0) };
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));

        let actual = inner.get_conversation(&id).await?;
        assert!(actual.is_none());
        Ok(())
    }
}
