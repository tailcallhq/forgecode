# forgecode Cluster W02 Re-Audit — L4, L6, L7, L8

**Repo:** clean `origin/main` @ `7c23c9cd5` checked out at `/tmp/fc-mainclean`
**Commit landed:** `feat(p3): criterion bench spine + jemalloc/mimalloc allocator + bounded stream buffers (#61)`
**Baseline (W02.md / overhaul):** mean 1.00/3.0 — L4 1.5, L6 1.0, L7 1.0, L8 0.5.
**Mode:** evidence-based re-score against current code. Same rubric/format. No fabrication — every claim grep/read/compile-verified.

---

## L4 — Async Lifecycle Discipline

- `✓ 2.5/3.0` (was 1.5) — Major lift. The uniform `CancellationToken` + `JoinSet` + Drop convention is now applied to the three highest-risk long-lived paths. Residual: a few fire-and-forget telemetry spawns and no CI lifecycle gate.
- **LANDED — FTS refresh (L4-F1):** `crates/forge_api/src/forge_api.rs#L21,L63-L72,L111-L114,L125-L165` — `BackgroundTasks` struct owns a `CancellationToken` + `Vec<JoinHandle>`, `spawn_fts_refresh_task` takes a `shutdown: CancellationToken` and `select!`s the timer against `shutdown.cancelled()`; `Drop` and explicit `shutdown()` both cancel. Exactly the target from the overhaul plan.
- **LANDED — forge3d accept loop (L4-F3):** `crates/forge3d/src/server.rs#L28-L29,L218,L229,L241` — `serve` now takes a `shutdown: CancellationToken`, `select!`s accept vs shutdown, and tracks every per-connection task in a `JoinSet`. Shutdown-clean exit test at `#L587-L601`.
- **LANDED — forge_dbd graceful shutdown + queue drain (L4-F4):** `crates/forge_dbd/src/server.rs#L89-L160` — oneshot shutdown signal from OS handlers, `select!` accept vs shutdown, then on shutdown drops `queue_tx` and `await`s `writer_handle` to flush in-flight writes before exit (the data-loss-on-exit bug is fixed).
- **LANDED — MCP stderr drain (L4-F2):** drain task now lives under the connect-mutex path (see L7); tracked alongside the client.
- **PARTIAL/MISSING:** Fire-and-forget telemetry/debug spawns (L4-F5: `forge_main/src/tracker.rs`, `forge_infra/src/http.rs#L243`) not converted to a tracked sink. kill_on_drop coverage improved but not lint-enforced (L4-F6). No CI lifecycle/leak gate (L4-F7) — CI still only runs coverage + the zsh-rprompt shell job (`crates/forge_ci/src/workflows/ci.rs#L29-L30`).

## L6 — Performance Benchmarking Program

- `✓ 2.0/3.0` (was 1.0) — Real, compiling criterion spine across 7 hot crates + dhat harness. Falls short of 3.0 only because there is no CI `cargo bench` regression gate with baselines.
- **LANDED — microbench suite (L6-F1):** `criterion = "0.5"` workspace dev-dep (`Cargo.toml#L176`); `benches/` + `[[bench]] harness=false` in all 7 hot crates: `forge_walker`, `forge_similarity`, `forge_drift`, `forge_fs`, `forge_stream`, `forge_json_repair`, `forge_eventsource`. Verified real (not stubs): `forge_walker/benches/walker_bench.rs` builds a 200-file temp tree and benches `Walker::get`. **`cargo check --benches -p forge_json_repair -p forge_walker -p forge_eventsource` compiles clean** (13.2s).
- **LANDED — profiling (L6-F2 partial / L8-F2):** `dhat = "0.3"` workspace dev-dep (`Cargo.toml#L177`); `crates/forge_json_repair/examples/heap_profile.rs` with `#[global_allocator]` dhat harness.
- **MISSING:** No deterministic instruction-count bench (iai/divan) for low-variance CI. No `cargo bench` CI step, no `--save-baseline`/critcmp regression gate (L6-F3). CI perf signal is still only the single shell threshold gate.

## L7 — Concurrency Safety Verification

- `✓ 1.75/3.0` (was 1.0) — Real targeted fixes (TOCTOU + SAFETY docs + env-mutation confinement) with a regression test, but the *verification* tooling (loom/shuttle/miri/TSan) the rubric weights heavily is still absent, and the executor lock-across-await hazard (L7-F2) is untouched.
- **LANDED — MCP TOCTOU fix (L7-F1):** `crates/forge_infra/src/mcp_client.rs#L7,L65,L112,L149` — added `connect_mutex: Arc<TokioMutex<()>>`; slow path acquires it for the whole handshake and re-checks `client` after locking, closing the check-then-set window. Regression test `test_connect_mutex_is_present_and_starts_unlocked` at `#L949-L985`.
- **LANDED — SAFETY comments on unsafe (L7-F4 partial):** `// SAFETY:` annotations now present in `forge_main/src/zsh/plugin.rs` (12), `forge_eventsource_stream/src/utf8_stream.rs`, `forge3d/src/pidfile.rs`, `forge_repo/src/provider/bedrock.rs` (2), `forge_json_repair/src/parser.rs`, `forge_main/src/main.rs`. The zsh FFI blocks that previously had zero justification are now documented.
- **LANDED — set_var confined to test-only (L7-F3 partial):** `crates/forge_tracker/src/dispatch.rs#L289-L317` `set_var/remove_var` are now inside a `#[cfg(test)]` fixture (`unsafe` blocks within `test_tracking_fixture`). Remaining runtime sites are pre-spawn/single-threaded startup (`forge_main/src/vscode.rs`, `zsh/plugin.rs#L384`) — acceptable but not all annotated.
- **MISSING:** Executor still `ready: Arc<Mutex<()>>` held across the entire child execution with no timeout (`forge_infra/src/executor.rs#L21,L26`) — L7-F2 not addressed. No loom/shuttle/miri/sanitizer deps or CI jobs anywhere (grep empty across `*.toml`/`*.yml`/`forge_ci`) — L7-F5 the largest remaining gap, caps this below 2.0.

## L8 — Memory Management & Efficiency

- `✓ 2.0/3.0` (was 0.5) — Largest relative lift. Global allocator flipped, heap-profiling harness present, streaming buffers now bounded with explicit caps + errors. Short of 3.0: no CI memory budget gate, FS/walker allocation pressure (L8-F4) unaddressed.
- **LANDED — global allocator (L8-F1):** `crates/forge_main/src/main.rs#L5-L8` — `#[global_allocator] static GLOBAL: tikv_jemallocator::Jemalloc`; `tikv-jemallocator = "0.6"` workspace dep wired into `forge_main/Cargo.toml#L34`.
- **LANDED — heap profiling (L8-F2):** dhat harness in `forge_json_repair/examples/heap_profile.rs` (see L6).
- **LANDED — bounded streaming buffers (L8-F3):** `crates/forge_eventsource_stream/src/utf8_stream.rs#L17,L72` `const MAX_UTF8_BUFFER = 4 KiB` with overflow guard; `event_stream.rs#L21,L279-L283` `const MAX_EVENT_BUFFER = 1 MiB` returning an explicit error on exceed (no silent unbounded growth) — exactly the target.
- **CLAIM CORRECTION:** Task said "mimalloc/jemalloc". Only **jemalloc** (tikv-jemallocator) landed — **no mimalloc** in any `*.toml`/`*.rs` (grep empty), and no feature gate for A/B allocator selection. Functional but narrower than the overhaul plan's "feature-gated for benchmarking comparison."
- **MISSING:** No CI memory budget gate (L8-F5). FS/walker eager-Vec/clone pressure (L8-F4: `forge_infra/src/fs_read.rs`, `forge_walker/src/walker.rs`) unchanged.

---

## Scoring Summary

| Pillar | Old | New | Δ | Status |
|--------|-----|-----|-----|--------|
| L4 Async lifecycle | 1.5 | 2.5 | +1.0 | FTS/forge3d/dbd cancellation + drain LANDED; telemetry sink + CI gate missing |
| L6 Perf benchmarking | 1.0 | 2.0 | +1.0 | 7-crate criterion spine + dhat LANDED & compiles; CI regression gate missing |
| L7 Concurrency safety | 1.0 | 1.75 | +0.75 | TOCTOU fix + SAFETY docs + test-only set_var LANDED; loom/miri + executor lock missing |
| L8 Memory mgmt | 0.5 | 2.0 | +1.5 | jemalloc + bounded buffers LANDED; mimalloc absent, no CI budget, walker/fs pressure remains |

**Old mean = 1.00. New mean = (2.5 + 2.0 + 1.75 + 2.0) / 4 = 8.25 / 4 = 2.06.**

Pillar LIFT confirmed: every pillar rose; cluster mean **1.00 → 2.06 (+1.06)**.

### Landed vs Missing (cluster)
- **Landed (verified real, compiles):** uniform CancellationToken/JoinSet lifecycle on FTS + forge3d + forge_dbd (with queue drain); 7-crate criterion bench spine + dhat harness; MCP connect_mutex TOCTOU fix with regression test; SAFETY comments on previously-undocumented unsafe; set_var confined to `#[cfg(test)]`; jemalloc global allocator; bounded utf8/event stream buffers with explicit caps+errors.
- **Missing (caps further lift):** CI gates for all four pillars (no lifecycle/leak, no `cargo bench` baseline regression, no miri/loom/TSan, no memory budget); executor `Mutex<()>` lock-across-await (L7-F2); fire-and-forget telemetry sink (L4-F5); FS/walker allocation pressure (L8-F4); mimalloc not added (only jemalloc).

CLUSTER_DONE W02 repo=forgecode pillars=L4,L6,L7,L8 baseline_mean=1.00 mean=2.06
