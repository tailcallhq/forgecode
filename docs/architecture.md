# Architecture — forgecode fork

## Homes & sync
```
official forge  →  ~/.forge/.forge.db  (write DB, owned)
                     ↓  poll 5s (mtime gate, ForgeAPI::spawn_upstream_sync_task)
helioslite      →  ~/.helioslite/.forge.writes.db  ←  import_forge_db (idempotent, row-level)
helioslite R/W  →  ~/.helioslite/* (also writes locally, union view)
```
`ConfigReader::forge_base_path()` vs `base_path()`, `is_helioslite_binary()` gates sync task (helioslite only, homes distinct).

## Daemon
```
ForgeConversationService --async--> DaemonConversationRepository --DbClient::send--> DbServer (Unix socket / NamedPipe)
                ↓ fallback only on Unavailable                     ↓ queue (mpsc 1024) → writer_task (batch 15ms/100, single Connection WAL)
         direct diesel pool (reads, FTS)
```
`try_daemon` → `DbClient::connect` (lazy OnceCell, fresh per send) → classify `Unavailable` (connect/spawn fail) vs `Indeterminate` (after send). `MutationV2` envelope carries `workspace_id` (outer) + inner `Option`.

## Code map
- `crates/forge_api/src/forge_api.rs` — `spawn_upstream_sync_task` + FTS refresh `BackgroundTasks`, `CancellationToken`
- `crates/forge_config/src/reader.rs` — home resolution
- `crates/forge_repo/src/daemon_repo.rs` — `DaemonConversationRepository`, `DaemonWriteOutcome`, `write_or_fallback`
- `crates/forge_dbd/src/{client,server,protocol,conversation_storage}` — frame codec (JSON), batch writer, `upsert_conversation(workspace_id)` (`persist_context` zstd)
- `crates/forge_ci` — workflow model → `.github/workflows/*.yml` (generated, `FORK_RELEASE_VERSION` input)

## Workflows
`ci.yml` (Build and Test, zsh perf, Draft Release with `version: 2.13.21-h.0.1.5`) + `release-drafter.yml` (standalone) share `release-drafter@SHA` + `config-name`. Scorecard-relevant workflows `perf-dashboard`, `otel-health`, `chaos-testing` are pinned + `contents: read`.

## Data flow (conversation)
`Conversation { id, workspace_id, context (zstd), metrics, parent_id }` → `ConversationRecord::new/new_ref` → `persist_context` → SQLite `conversations` (`conversation_id PK`, `workspace_id`, `context`, `context_zstd`, `is_compressed`, `message_count`, `updated_at` via `strftime`). `conversations_all` view unions `~/.forge` + `helioslite_home`.

## Threat / correctness
- No replay on `Indeterminate` → no duplicate rows after ack loss.
- Workspace scoping on `Delete`/`Upsert` prevents cross-workspace visibility (Windows `--directory` canonicalization).
- Single writer avoids WAL writer starvation; `busy_timeout 5s` + `journal_mode=WAL`.
