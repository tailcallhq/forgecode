# ADR 004: P3 Single-Writer Daemon (`forge_dbd`)

**Date:** 2026-08-18
**Status:** Accepted — Phase 1 done (single writer shipped; WAL `journal_mode=WAL` + `synchronous=NORMAL` + `busy_timeout=5s`; idle shutdown 300s; daemon owns all conversation writes)
**Deciders:** KooshaPari, Forgecode Maintainers
**Tags:** P3, daemon, persistence, concurrency, SQLite, WAL

---

## Context

Forgecode persists conversations to SQLite (`~/.forge/.forge.db` for upstream
`forge`, `~/.helioslite/.forge.writes.db` for the `helioslite` fork binary).
The storage path uses Diesel + r2d2 with `journal_mode=WAL` and
`busy_timeout=5s`. Under the P0/P1 fork work the pool was shared by every
client process; each `forge`/`helioslite` invocation opened its own
`SqliteConnection` and issued writes concurrently.

Three failure modes were observed at scale (3047+ workspace tests, daemon
integration tests):

1. **WAL writer starvation.** SQLite allows concurrent readers but serialises
   writers at the WAL. With N CLI/TUI processes racing on `conversations`
   (`ON CONFLICT(conversation_id) DO UPDATE`), `SQLITE_BUSY` spikes under
   burst writes. `busy_timeout` masks the symptom but increases P95 latency
   and, under sustained pressure, surfaces as `database is locked`.

2. **Cross-workspace visibility.** `workspace_id` (hash of client cwd via
   `WorkspaceHash::new` with zero-seed `DefaultHasher`) must scope every
   mutation. Direct-pool writes relied on each caller to supply the correct
   `workspace_id`; a stale or missing value leaked rows across workspaces.
   Windows `--directory` canonicalisation made the bug path-dependent.

3. **No idempotent replay boundary.** A client that sent bytes to SQLite and
   then lost its connection had no way to distinguish "never sent" from
   "sent but ack lost". Replaying the mutation risked duplicate
   `context_zstd`/`message_count` updates; not replaying risked data loss.
   The existing fallback was unconditional and therefore unsafe.

P2 (resilience/observability) closed the sync invariant
(`~/.forge` owned by upstream, `helioslite` polls `mtime` every 5s via
`ForgeAPI::spawn_upstream_sync_task` and `import_forge_db` idempotently). P3
must close the writer invariant: exactly one writer owns the database file at
any time, all mutations flow through it, and delivery certainty is explicit.

Constraints:

- Must run on Unix and Windows (Unix domain socket vs Windows named pipe).
- Must not require kernel file locks (NFS/AV interference on Windows).
- Must preserve the existing `conversation_storage::persist_context` envelope
  (zstd legacy `context_zstd`/`is_compressed`/`context` + `message_count`
  atomic upsert) and `conversations_all` union view.
- Must degrade gracefully when the daemon is absent (fresh install, `cargo
  test` in-memory pool).

---

## Decision

We will introduce `forge_dbd` — a **single-writer session daemon** — as the
exclusive writer to the conversation SQLite database. All conversation
mutations are routed through the daemon; reads and FTS continue to use the
direct Diesel pool.

### Architecture

```
ForgeConversationService
        │ async
        ▼
DaemonConversationRepository ──DbClient::send──► DbServer
        │ fallback only on Unavailable           │ mpsc 1024
        │ direct pool (reads, FTS)               ▼
        └─────────────────────────────────► writer_task
                                              batch 15ms/100 rows
                                              single rusqlite Connection (WAL)
                                              ↓
                                          SQLite conversations
                                          (conversation_id PK, workspace_id,
                                           context, context_zstd,
                                           is_compressed, message_count,
                                           updated_at strftime)
```

### Transport

- **Unix:** `~/.forge/.forge.db.sock` (socket file alongside the DB).
- **Windows:** named pipe `\\.\pipe\forge-dbd-<sanitised-socket-path>` derived
  deterministically: every char not in `[A-Za-z0-9.\-]` folded to `-`.
- **Frame codec:** `u32 LE length prefix + JSON body` (`write_frame` /
  `read_frame` in `crates/forge_dbd/src/protocol.rs`). JSON chosen over
  bincode as the P3 debugging-friendly codec — `Conversation`/`Metrics` use
  `skip_serializing_if` extensively, which positional bincode cannot round-trip.
- **Client:** `DbClient { socket_path }` with lazy `OnceCell` transport, fresh
  connection per `send`. `DbClient::connect` then `DbClient::send` per request.

### Lifecycle and spawning

- First routed write calls `try_daemon` → `DbClient::connect`. If the socket is
  absent, the client spawns `forge_dbd` from `FORGE_DBD_BIN` or `forge_dbd` on
  `PATH`, guarded by a process-wide `SPAWN_ATTEMPTED: AtomicBool` (once per
  process; subsequent failures do not re-spawn). The client polls up to 2s for
  the socket to appear before classifying the attempt.
- Server `main.rs` binds the socket/pipe, spawns `writer_task`, serves
  `tokio::spawn` per connection. `mpsc::channel(1024)` bounds back-pressure.
  The daemon shuts down after `idle_timeout` 300s with no connections (configurable via `DbServer::new_with_idle`; default `Duration::from_secs(300)`, reset on each connect/disconnect).
- Health: `writer_task` owns the sole `Connection`; on `Drop` the socket file
  is removed (Unix) / pipe closed (Windows).
- **SQLite pragmas (writer only):** `PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;` plus `busy_timeout(5s)` on the single `rusqlite::Connection` (`crates/forge_dbd/src/server.rs::open_writer_connection`). WAL gives single-writer / concurrent-reader semantics; `NORMAL` is safe under WAL with the daemon as sole writer.

### Protocol

```rust
pub const MUTATION_PROTOCOL_VERSION: u16 = 2;
pub const LEGACY_PROTOCOL_VERSION: u16 = 1;

pub enum Request {
    Ping,
    MutationV2 { workspace_id: i64, mutation: ConversationMutation },
}

pub enum ConversationMutation {
    UpsertConversation { conversation: Conversation, workspace_id: Option<i64> },
    UpsertRef { conversation_id: ConversationId, ref_id: String, workspace_id: Option<i64> },
    UpdateParentId { conversation_id: ConversationId, parent_id: Option<ConversationId>, workspace_id: Option<i64> },
    Delete { conversation_id: ConversationId, workspace_id: Option<i64> },
}

pub enum Response { Ack, Error { message: String }, Health(HealthStatus) }
pub struct HealthStatus { protocol_version: u16, uptime_secs: u64, queue_depth: usize, db_reachable: bool }
```

Rules:

- **MutationV2 is the only accepted mutation.** Legacy variants
  `UpsertConversation/Ref/UpdateParentId/Delete` (without envelope) are
  rejected with `legacy unscoped mutation`. This forces workspace scoping.
- **Outer `workspace_id` is authoritative.** Inner `Option<i64>` is carried
  for wire compatibility but the server ignores it (`..` pattern). The value
  is `self.inner.workspace_id()` — hash of the *client* cwd.
- **Handshake:** clients issue `Ping → Health{protocol_version, ...}` before
  the first mutation. A `Health` with `protocol_version < 2` aborts the
  mutation with a version-mismatch error.
- **Storage:** `conversation_storage::persist_context` writes the zstd envelope
  atomically (`ON CONFLICT(conversation_id) DO UPDATE SET context_zstd,
  is_compressed, context, message_count, workspace_id, updated_at`). No
  `INSERT OR REPLACE`; row identity is stable.

### Delivery certainty

```rust
pub enum DaemonWriteOutcome { Ack, Unavailable(anyhow::Error), Indeterminate(anyhow::Error) }
pub enum DbClientSendError { Unavailable(anyhow::Error), Indeterminate(anyhow::Error) }
```

- `Unavailable` — transport connect failed or `SPAWN_ATTEMPTED` already set and
  socket absent; no bytes were sent. Caller may safely fallback to `inner`
  direct write via `write_or_fallback`.
- `Indeterminate` — connection established and at least one frame was written;
  daemon may have enqueued the mutation. Caller must **not** replay; error is
  surfaced to the caller (`anyhow` with `daemon indeterminate write` context).
- `Ack` — `Response::Ack` received; mutation is durable.

`DaemonConversationRepository::write_or_fallback` enforces the trichotomy. No
caller may match on `DbClientSendError` and retry `Indeterminate`.

### Batching

`writer_task` batches up to 100 mutations or 15ms (whichever fires first) and
executes them in a single `Connection` transaction. Batch failures fail each
item individually with `Response::Error`; partial-batch durability is not
assumed.

### Code map

- `crates/forge_dbd/src/protocol.rs` — `MUTATION_PROTOCOL_VERSION`, frame codec, `Request`/`Response`/`HealthStatus`, Windows pipe name derivation.
- `crates/forge_dbd/src/client.rs` — `DbClient`, `DbClientSendError`, `write_frame`/`read_frame`, `SPAWN_ATTEMPTED` guard, 2s spawn poll.
- `crates/forge_dbd/src/server.rs` — `DbServer`, `writer_task` (mpsc 1024, batch 15ms/100, single WAL `Connection`), `upsert_conversation(workspace_id)`.
- `crates/forge_dbd/src/conversation_storage.rs` — `persist_context` (zstd envelope, conflict upsert).
- `crates/forge_repo/src/daemon_repo.rs` — `DaemonConversationRepository`, `DaemonWriteOutcome`, `write_or_fallback`, tests `spawn_is_attempted_once_then_falls_back` and `does_not_fallback_after_daemon_records_request_then_loses_ack`.
- `crates/forge_api/src/forge_api.rs` — `spawn_upstream_sync_task` (5s poll, `FORGE_SYNC_INTERVAL_SECS`, `FORGE_SYNC_DISABLED=1`) remains read-side only; daemon does not own sync.
- `crates/forge_config/src/reader.rs` — `forge_base_path()` vs `base_path()` unchanged.

---

## Alternatives considered

| Alternative | Verdict |
|-------------|---------|
| **File locking (`flock`/`LockFileEx`)** | Rejected — unreliable on NFS/AV, no cross-workspace scoping, still requires busy-retry. |
| **SQLite `BEGIN IMMEDIATE` + retry** | Rejected — hides contention, increases tail latency, no delivery certainty. |
| **Per-workspace DB files** | Rejected — fragments `conversations_all` view, complicates FTS5 and sync union. |
| **bincode codec** | Rejected — positional format cannot round-trip `skip_serializing_if` domain types; fails encode→decode of same value. |
| **TCP localhost daemon** | Rejected — firewall/AV prompts on Windows, port collision; UDS/pipe is OS-native. |

---

## Consequences

### Positive

- Single writer eliminates WAL contention; P95 write latency drops from
  `busy_timeout`-dominated to batch-commit dominated (~15ms).
- Explicit `Unavailable` vs `Indeterminate` prevents both data loss (never
  suppress `Indeterminate`) and duplicate writes (never replay `Indeterminate`).
- Workspace scoping is structural: old binaries cannot write without envelope;
  server error surfaces immediately in CI.
- JSON frames are debuggable (`RUST_LOG=forge_dbd=debug` logs the mutation).
- Windows named pipe derivation is deterministic without extra config.

### Negative

- Additional process (`forge_dbd`) to ship (release matrix now includes
  `forge_dbd` on every platform; `forge_ci` workflow model emits it).
- First-write latency includes up to 2s spawn poll when daemon is cold.
- Daemon crash after `Indeterminate` leaves the mutation in unknown state;
  caller must report error (future work: idempotency key).

### Neutral

- Reads/FTS bypass the daemon (direct pool, unchanged).
- `SPAWN_ATTEMPTED` once-per-process means a transient daemon failure in a
  test process causes remaining tests in that process to use direct writes;
  integration tests use isolated temp socket paths to avoid cross-talk.

---

## Validation

- `cargo test --workspace` (3047+ tests) including daemon unit tests:
  `spawn_is_attempted_once_then_falls_back`,
  `does_not_fallback_after_daemon_records_request_then_loses_ack`.
- `heliosdoctor` validates fork version `2.13.21-h.0.1.x`, home separation, and
  FTS reachability after daemon migration.
- `gh api repos/KooshaPari/forgecode/releases/latest` → 36 fork assets
  (including `forge_dbd`) with `prerelease: false`.

---

## References

- `docs/requirements.md` §3 (Daemon P3 single-writer)
- `docs/architecture.md` (Daemon diagram, code map)
- `crates/forge_dbd/README.md`
- `crates/forge_ci/src/jobs/release_draft.rs::FORK_RELEASE_VERSION`
- `docs/31-pillar-scorecard.md` (Scorecard `contents: read`, SHA pins via ratchet)

