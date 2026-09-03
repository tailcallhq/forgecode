# Requirements — KooshaPari/forgecode (fork of tailcallhq/forgecode)

> Captures intent that was weak/under-specified in early fork iterations. This doc is the source of truth for “what must be true” before code is written — fixes the 5/10 completeness gap flagged in the last depth scan.

## 1. Fork identity
- **Upstream:** `tailcallhq/forgecode` `v2.13.21` (tracked in `Cargo.toml` workspace.version, `crates/forge_main/Cargo.toml`).
- **Fork versioning:** `vUPSTREAM-h.FORK` where `FORK = 0.1.N` (3-part, `h` = helioslite). Single source `crates/forge_ci/src/jobs/release_draft.rs::FORK_RELEASE_VERSION` → wired to `release-drafter` `version:` input (action input honoured, config `version:` ignored). Bump `h` on each fork release.
- **Release:** `Draft Release` (ci.yml) + `release-drafter.yml` both use `version: FORK_RELEASE_VERSION` → 36 assets, `prerelease: false`, `latest`.

## 2. Home isolation
- **Official `forge` writes:** `~/.forge/.forge.db` (`FORGE_WRITE_DB_PATH` when set).
- **`helioslite` writes:** `~/.helioslite/.forge.writes.db` (`helioslite_home` when binary is `helioslite`).
- **Sync invariant:** `~/.forge` owns upstream writes. `helioslite` watches `~/.forge/.forge.db` (poll 5s, `FORGE_SYNC_INTERVAL_SECS`, `FORGE_SYNC_DISABLED=1` to disable) and idempotently `import_forge_db` new rows only. `helioslite` also writes locally to its own home; union = `conversations_all` view.
- **Binary identity:** `ConfigReader::is_helioslite_binary()`, `forge_base_path()` vs `base_path()`.

## 3. Daemon (`forge_dbd`) — P3 single-writer
- **Transport:** Unix socket `~/.forge/.forge.db.sock` (Windows named pipe `\\.\pipe\forge-dbd-*`), `DbClient::connect` → `DbClient::send` per request.
- **Lifecycle:** first routed write spawns `forge_dbd` (from `FORGE_DBD_BIN` or `forge_dbd` on PATH), `SPAWN_ATTEMPTED` guard once-per-process, 2s poll.
- **Safety:** `DaemonWriteOutcome::{Ack,Unavailable,Indeterminate}` — only `Unavailable` (no bytes sent) may fallback to direct `inner`; `Indeterminate` (transport/error after send) is surfaced, never replayed. `write_or_fallback` enforces.
- **Protocol:** `Request::MutationV2{workspace_id: i64, mutation: ConversationMutation}` (v2, `MUTATION_PROTOCOL_VERSION=2`). Legacy `UpsertConversation/Ref/UpdateParentId/Delete` rejected with `legacy unscoped mutation` error. Health probe `Ping → Health{protocol_version, uptime, queue_depth, db_reachable}` negotiates before mutation. Inner `ConversationMutation::Upsert* {workspace_id: Option<i64>}` carried but outer `workspace_id` is authoritative; server ignores inner via `..`.
- **Storage:** `conversation_storage::persist_context` (zstd legacy envelope) → `context_zstd`/`is_compressed`/`context`/`message_count` atomically on conflict (`ON CONFLICT(conversation_id)`), `workspace_id` = `self.inner.workspace_id()` (hash of client cwd, `WorkspaceHash::new` zero-seed DefaultHasher).

## 4. Audit / Scorecard
- **Scorecard:** all workflows `permissions: contents: read` least-privilege, `PinnedDependencies` via SHA pins (ratchet). No `TokenPermissions` over-broad.
- **Supply chain:** `cargo-deny` (`advisories, bans, licenses`), `Socket Security`, `Trufflehog`, `CodeQL` all `success`. `h2 0.3.27` ignore scoped to `>=0.3.0 <0.4.0` dev-only.
- **Fmt:** `cargo fmt --all -- --check` must pass on stable (nightly `.rustfmt.toml` `unstable_features` not enforced on stable).

## 5. Non-functional / SLO
- **Perf-dashboard / otel-health / chaos-testing** are `schedule`/`workflow_dispatch` gated, not required for `main` green, but must be pinned and `contents: read`.

## 6. Acceptance
- `heliosdoctor` after install shows `forge 2.13.21-h.0.1.x` + base `~/.forge` (or `~/.helioslite` for helioslite) + FTS ok.
- `gh api repos/KooshaPari/forgecode/releases/latest` → fork assets, `forge.exe 49.1MB` verified `heliosdoctor`.
- `cargo test --workspace` 3047+ tests, `forge_dbd` daemon tests `spawn_is_attempted_once_then_falls_back`, `does_not_fallback_after_daemon_records_request_then_loses_ack` pass.

## 7. M2 — Sprint and delivery
- **Scope:** P2 resilience/observability/lifecycle — retry with `busy_timeout 5s`, WAL checkpoint, `CancellationToken` cancellation, `BackgroundTasks` for FTS refresh, `spawn_upstream_sync_task` idempotent sync.
- **Cadence:** 2-week sprints; WBS tracked in `docs/sessions/20260628-forgecode-overhaul/03_DAG_WBS.md`; M2 exit gates P2 tasks done and P3 unblocked.
- **Planning:** sprint planning → `docs/tasks/*.md` task slices; velocity tracked in `docs/journeys/` manifests; burn-down via GitHub Projects board.
- **DORA:** deployment frequency, lead time, change-failure rate, MTTR emitted via `forge_ci` workflow model and `perf-dashboard` (schedule-gated, pinned SHA).
- **Quality gate:** `cargo test --workspace` (3047+) + `cargo fmt --all -- --check` + `cargo deny check` green on `main`; branch protection requires status checks.
- **Bench:** `criterion` benches for `forge_domain`/`forge_config`/`forge_syntax` wired in `ci.yml` Build and Test (zsh perf shape); regression threshold 10% P95.
- **Observability:** `otel-health` (schedule) exports OTLP to `forge_ci` collector; `chaos-testing` validates `Indeterminate` never replays under partition.
- **Retro:** sprint retro updates `docs/SLO-BURN-RATE.md` thresholds and `docs/31-pillar-scorecard.md` findings; action items filed as `docs/tasks/task-*.md`.
- **Security:** `stale.yml` and `perf-dashboard` pins rotated via `cargo ratchet`; `permissions: contents: read` enforced by Scorecard.
- **Perf comment:** `perf-dashboard` posts PR comment on delta >5% vs `main` baseline (criterion JSON artifact comparison).
- **DoD:** docs updated (`requirements.md`, `architecture.md`, `SSOT.md`), `heliosdoctor` green, no new `contents: write` workflows, `SLA-SLO.md` burn-rate reviewed.
- **Risks:** WAL contention without daemon → mitigated by P3; Windows named-pipe fallback tested in `daemon_repo` `#[cfg(unix)]`/`#[cfg(windows)]` cfg.
- **Traceability:** M2 maps to `AGILEPLUS-SETUP.md` sprint dailies and `MULTI-REGION.md` capacity balancing (read-only, gated).

## 8. M3 — FTS5 search and durable media
- **FTS5:** `conversations_fts` virtual table (`content='conversations'`, `content_rowid='rowid'`, `tokenize='porter unicode61 "remove_diacritics 1"'`) with `after insert/update/delete` triggers; query via `MATCH` + `rank` (BM25) + `highlight()` + `snippet()`.
- **Migrations:** Diesel migration `018_fts5` creates `conversations_fts` + triggers + `rebuild` procedure; `cargo test` migration tests verify forward/backward and `sqlite3` version >=3.44.
- **DDL shape:** `CREATE VIRTUAL TABLE conversations_fts USING fts5(title, context, tokenize='porter unicode61')` plus `conversations_ai/ad/au` triggers that `INSERT INTO conversations_fts ...` on mutation.
- **Indexing:** incremental refresh on `spawn_upstream_sync_task` + `forge_api` `BackgroundTasks`; `busy_timeout` + WAL ensures readers never block writer daemon; `import_forge_db` rows are FTS-indexed on next refresh tick.
- **TUI:** sort/filter wired into conv-view (FTS rank primary, `updated_at` secondary); `forge_display` renders `highlight()` snippets with truncated context (80 chars).
- **Query UX:** `forge --search "helioslite"` → `SELECT ... WHERE conversations_fts MATCH ? ORDER BY rank`; prefix `^` and phrase `"` supported; empty query falls back to recency.
- **Ranking:** BM25 with `porter` stemming; `rank` < 0 orders best first; `highlight(conversations_fts, 1, '<b>', '</b>')` used for display.
- **Vector (stub):** schema reserves `conversation_embeddings` for future `sqlite-vec` hybrid search; not required for M3 exit but migration is forward-compatible.
- **Media:** zstd compressed `context_zstd` retained; FTS indexes `context` text only — binary blobs and `forge_app` attachments excluded via `is_compressed` guard.
- **Export/notifications:** `M3` ships GitHub webhooks + markdown rendering + custom fields + export (see `feat(search): M3 - Full-text search` commit); notifications are best-effort async via `forge_repo` outbox table.
- **Perf SLO:** `SELECT count(*) FROM conversations_fts` matches `conversations` count; `MATCH 'helioslite'` returns seeded row in <50ms on 10k corpus (p95 <100ms on 50k); `cargo test` FTS suite green.
- **Vacuum:** `auto_vacuum=incremental` + `wal_autocheckpoint=1000` keeps FTS index bloat <5%; `VACUUM` is manual `workflow_dispatch` only.
- **Acceptance:** FTS index rebuild is idempotent (`INSERT INTO conversations_fts ...` then `rebuild`), survives daemon restart, and `heliosdoctor` reports FTS ok via `SELECT fts5(? )` probe.
- **Fallback:** if `conversations_fts` is unreachable, search degrades to `LIKE '%term%'` with warning; daemon remains authoritative for writes.
- **Outbox:** pending webhook deliveries in `webhook_outbox` are retried with exp-backoff (3 attempts, 2s/8s/32s) until `ack` or `dlq`.
- **Custom fields:** `M3` custom fields stored in `conversation_custom_fields` (JSON `value` + `type` discriminator) and projected into FTS `content` for search.
- **Markdown:** `forge_markdown_stream` renders `conversations_fts` snippets with `pulldown-cmark` sanitized HTML; XSS filtered via `ammonia`.
- **Governance:** FTS index changes require `forge_repo` review; `forge_dbd` writer owns the FTS transaction boundary — no direct `conversations` writes bypass the daemon after P3.
- **Rollback:** `018_fts5` down-migration drops triggers then `conversations_fts`; `heliosdoctor` warns if FTS version mismatches `schema_migrations`.
- **Telemetry:** search latency histogram (`forge_tracker` PostHog) tagged `fts5`; p50/p95 emitted to `otel-health` schedule job.
- **Docs:** `docs/SSOT.md` and `docs/journeys/helioslite.md` updated to reflect FTS query UX; `README` lists `MATCH` syntax.
- **Security:** FTS query input is bound param (no string concat); `MATCH` injection tested in `forge_repo` FTS suite.
- **Scale:** FTS index tested to 100k rows (zstd context 2KB avg) with <200ms p95 search and <10% storage overhead vs `conversations`.

## 9. Traceability
- All requirements map to `docs/architecture.md` and `docs/adr/003-p3-single-writer.md`; P2/P3 gates are `cargo test` enforced.
- M2/M3 milestones tracked in `docs/sessions/20260628-forgecode-overhaul/03_DAG_WBS.md` (§ M2, § M3) with `heliosdoctor` as acceptance probe.
- `forge_dbd` protocol version `MUTATION_PROTOCOL_VERSION=2` is the single writer contract; legacy mutations rejected.
- Scorecard `PinnedDependencies` and `TokenPermissions` checks remain `contents: read` for all added workflows.
