# P3 Single-Writer Daemon

## Scope

Phenotype-org addition for the tailcallhq/forgecode fork.

This design keeps the existing \`.forge.db\` file and schema intact while
collapsing many concurrent forge processes into a single SQLite writer daemon.
There is no data migration.

## Problem Statement

Forge currently allows multiple processes to open the same SQLite database and
issue writes independently. Even with WAL mode, concurrent writers still contend
on the single SQLite write lock. The hot path here is the per-turn conversation
upsert flow, which is called frequently enough that contention becomes the
dominant bottleneck.

## Proposed Architecture

### Daemon ownership

Introduce a new \`forge-dbd\` daemon that owns the only read/write SQLite
connection for \`.forge.db\`.

### Client access model

Clients connect to the daemon over a Unix domain socket at:

\`~/.forge/.forge.db.sock\`

Recommended split:

- Reads stay direct through the existing repository path.
- Writes go through the daemon.

Rationale:

- WAL already allows concurrent readers.
- Reads are latency-sensitive and do not benefit from an extra hop.
- Writes benefit from centralized batching and serialization.

An alternate fully proxied mode is possible later, but is not required for this
phase.

### Direct mode fallback

If the daemon is not desired, unavailable, or explicitly disabled, the client
may fall back to direct SQLite access using the current code path.

This is a mode switch only. It does not change schema, file layout, or data.

## Wire Protocol

The daemon protocol is a request/response enum exchanged over a length-prefixed
frame.

Encoding options:

- \`bincode\` for the default compact wire format.
- JSON as a debugging-friendly alternate.

The protocol mirrors repository write operations, not internal SQL details.
Reads are intentionally omitted from the daemon contract in the recommended
mode.

### Request variants

- \`UpsertConversation\`
- \`UpsertConversationRef\`
- \`UpdateParentId\`
- \`DeleteConversation\`
- \`OptimizeFts\`
- \`RefreshFts\`
- \`CheckpointWal\`

### Response variants

- \`Ack\`
- \`Error\`

The request payloads should carry the concrete domain types used by the repo
layer so the client does not need to re-encode business meaning in ad hoc
structures.

## Lifecycle

### Startup

1. Client checks whether the socket exists and accepts connections.
2. If the socket is live, the client connects.
3. If not, the first client attempts to spawn \`forge-dbd\`.
4. Startup is guarded by an advisory lock so only one process performs bind and
   bootstrap.

### Runtime

- The daemon opens the SQLite database once and keeps the single writer
  connection.
- Incoming write requests are queued and executed on that connection.
- The WAL checkpointer that previously lived in P1 moves into the daemon.
  Since the daemon is now the only writer, checkpointing is easier to schedule
  and reason about centrally.

### Shutdown

- The daemon exits after the last client disconnects and the idle timeout
  expires.
- A stale socket is unlinked and recreated on next startup.

## Batching Strategy

The main amplification win is transaction batching.

The daemon should coalesce many write requests that arrive within a short window
into a single SQLite transaction. The conversation upsert path is the primary
target.

Batching goals:

- Reduce lock churn.
- Reduce fsync frequency.
- Preserve per-request acknowledgement semantics.

Operational shape:

- Accumulate requests for a short debounce window.
- Flush on timeout or when the queue reaches a threshold.
- Execute all batched writes inside one transaction.
- Return a response per request.

This is especially useful for the per-turn conversation persistence burst, where
multiple upserts may arrive back-to-back during a single agent turn.

## Client-Side Swap

The follow-up implementation will replace the write branch of
\`ConversationRepositoryImpl::run_with_connection\` with:

1. Serialize the repository operation into a protocol request.
2. Send it over the socket.
3. Await acknowledgement or error.

The \`ConversationRepository\` trait surface stays unchanged, so callers do not
need to change.

## Failure Modes

### Daemon crash

- Clients should reconnect.
- If needed, the first reconnecting client can respawn the daemon.

### Stale socket

- Detect failed connect or failed handshake.
- Unlink the stale path.
- Recreate the socket and respawn the daemon if required.

### Batch failure

- If a transaction fails, return the error for each request in the affected
  batch.
- The daemon remains responsible for mapping the database failure back to the
  request envelope.

### Writer lock contention inside the daemon

- This should be rare because the daemon owns the only writer connection.
- Any internal retry policy should remain local to the daemon process.

## Non-Goals

- No schema migration.
- No table rewrite.
- No repository trait redesign.
- No client-side caller changes in this task.
- No full daemon implementation in this scaffold.

## Scope Boundaries For This Task

This task only adds:

- design documentation
- \`crates/forge_dbd/\` scaffold
- workspace membership entry

The existing repository crates remain untouched.
