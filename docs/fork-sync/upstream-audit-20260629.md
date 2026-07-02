# Upstream Fork Sync Audit — 2026-06-29

**Repository**: `KooshaPari/forgecode` (fork of `tailcallhq/forgecode`)
**Audit date**: 2026-06-29
**Branch**: `main`
**Upstream ref**: `upstream/main` (`git@github.com:tailcallhq/forgecode.git`)
**Merge base**: `706802e43ddf70a9b1caabf2271e06199eb349a2`

---

## Executive Summary

Our fork is **11 commits behind** upstream/main. All 11 are Renovate-generated routine dependency bumps with no business logic changes. However, the structural divergence is **substantial**: our fork has accumulated many fork-specific commits adding features, CI hardening, security tooling, documentation, and new crate modules that upstream does not have. Upstream has simultaneously been simplifying their codebase — removing modules that our fork depends on.

**Net diffstat across targeted crates** (`forge_main`, `forge_api`, `forge_app`, `forge_domain`):
- 46 files changed, 192 insertions(+), 4897 deletions(-)

The `+192` lines represent upstream changes we are missing (mostly version bumps). The `-4897` lines represent code our fork has that upstream removed or does not have — meaning any merge attempt would face **significant conflicts** in core areas.

---

## 11 Unmerged Upstream Commits (all Renovate bot)

| # | Commit | Description | Scope | Impact |
|---|--------|-------------|-------|--------|
| 1 | `09e836a3e` | chore(deps): update rust crate config to v0.15.25 (#3574) | workspace | Lockfile + Cargo.toml |
| 2 | `315e5d0e4` | chore(deps): update dependency @ai-sdk/google-vertex to v5.0.1 (#3575) | npm | package-lock.json |
| 3 | `8c9ee12da` | chore(deps): update dependency ai to v7.0.3 (#3576) | npm | package-lock.json |
| 4 | `0e48954e6` | chore(deps): update rust crate posthog-rs to v0.14.2 (#3577) | workspace | Cargo.toml + Cargo.lock |
| 5 | `aef1ab30b` | chore(deps): update dependency @ai-sdk/google-vertex to v5.0.2 (#3578) | npm | package-lock.json |
| 6 | `fbd59728b` | chore(deps): update dependency ai to v7.0.4 (#3579) | npm | package-lock.json |
| 7 | `85d830d0b` | chore(deps): update rust crate open to v5.3.6 (#3587) | forge_tracker | Cargo.toml |
| 8 | `d5fc94bb9` | chore(deps): update rust crate indicatif to v0.18.5 (#3589) | workspace | Cargo.toml |
| 9 | `80b803022` | fix(deps): update rust crate posthog-rs to 0.15.0 (#3590) | workspace | Cargo.toml + Cargo.lock |
| 10 | `58c059581` | chore(deps): update dependency @ai-sdk/google-vertex to v5.0.3 (#3593) | npm | package-lock.json |
| 11 | `a282e68eb` | chore(deps): update dependency ai to v7.0.6 (#3594) | npm | package-lock.json |

**Files touched by these commits** (upstream from merge-base):
- `Cargo.toml` — 2 insertions, 2 deletions (dep version bumps)
- `Cargo.lock` — lockfile refresh
- `crates/forge_tracker/Cargo.toml` — 1 dep bump on `open` crate
- `package-lock.json` — npm dep refresh

**Risk**: Low. These are routine dep bumps with no API changes.

---

## Targeted Crate Diffstat

```
crates/forge_api/Cargo.toml                        |  10 +-
crates/forge_api/src/api.rs                        |  74 --
crates/forge_api/src/forge_api.rs                  | 198 +----
crates/forge_app/Cargo.toml                        |   3 +-
crates/forge_app/src/agent.rs                      |  18 -
crates/forge_app/src/agent_executor.rs             |  10 +-
crates/forge_app/src/hooks/doom_loop.rs            |   4 -
crates/forge_app/src/lib.rs                        |   1 -
crates/forge_app/src/llm_summarizer.rs             | 253 -------
crates/forge_app/src/orch.rs                       | 116 +--
crates/forge_app/src/services.rs                   | 166 -----
crates/forge_app/src/tool_executor.rs              |   1 -
crates/forge_app/src/tool_registry.rs              |  18 +-
crates/forge_domain/Cargo.toml                     |   3 +-
crates/forge_domain/src/auth/auth_token_response.rs|  63 +-
crates/forge_domain/src/auth/new_types.rs          | 163 +----
crates/forge_domain/src/compact/adaptive_eviction.rs| 273 -------
crates/forge_domain/src/compact/compact_config.rs  |  82 +--
crates/forge_domain/src/compact/history.rs         | 172 -----
crates/forge_domain/src/compact/importance.rs      | 328 ---------
crates/forge_domain/src/compact/metrics.rs         | 335 ---------
crates/forge_domain/src/compact/mod.rs             |  17 +-
crates/forge_domain/src/compact/prefilter.rs       | 319 --------
crates/forge_domain/src/compact/strategy.rs        | 108 ---
crates/forge_domain/src/conversation.rs            |  73 --
crates/forge_domain/src/intent.rs                  | 133 ----
crates/forge_domain/src/lib.rs                     |   4 -
crates/forge_domain/src/repo.rs                    | 246 -------
crates/forge_domain/src/telemetry.rs               |  56 --
crates/forge_domain/src/tools/call/context.rs      |  46 +-
crates/forge_main/Cargo.toml                       |   4 +-
crates/forge_main/src/cli.rs                       |  16 +-
crates/forge_main/src/conversation_selector.rs     | 259 ++-----
crates/forge_main/src/error.rs                     |  73 --
crates/forge_main/src/info.rs                      |  19 +-
crates/forge_main/src/input.rs                     |  11 -
crates/forge_main/src/main.rs                      |  26 +-
crates/forge_main/src/model.rs                     | 158 ----
crates/forge_main/src/state.rs                     | 169 +----
crates/forge_main/src/terminal/mod.rs              | 166 -----
crates/forge_main/src/ui.rs                        | 801 ++-------------------
crates/forge_main/src/update.rs                    |   2 +-
 46 files changed, 192 insertions(+), 4897 deletions(-)
```

---

## Detailed Crate Analysis

### `crates/forge_main/` — 13 files, ~18 insertions, ~1575 deletions

**Upstream changes we lack**:

| File | Δ | Analysis |
|------|---|----------|
| `Cargo.toml` | +2/-4 | Removed `tikv-jemallocator` dep; version reverted to `0.1.0` |
| `cli.rs` | +5/-9 | Removed `IsTerminal` check; upstream assumes TTY always interactive |
| `main.rs` | +5/-21 | Removed jemalloc, removed tokio ctrl-c handler |
| `info.rs` | +7/-13 | Removed `parent_id` breadcrumb, removed `Conversation` fields |
| `input.rs` | —/11 | Removed `clear_screen()` method |
| `update.rs` | +1/-1 | Update URL: `KooshaPari` → `tailcallhq` |
| `model.rs` | —/158 | **Entire file removed** upstream |
| `error.rs` | —/73 | **Entire file removed** upstream |
| `terminal/mod.rs` | —/166 | **Entire module removed** upstream |
| `ui.rs` | ~0/801 | Massively simplified upstream |
| `state.rs` | ~0/169 | Simplified state management |
| `conversation_selector.rs` | ~0/259 | Simplified conversation selection |

**Merge risk**: **HIGH**. Upstream deleted 3 modules, `ui.rs` heavily diverged.

---

### `crates/forge_api/` — 3 files, ~10 insertions, ~280 deletions

| File | Δ | Analysis |
|------|---|----------|
| `Cargo.toml` | +5/-9 | Removed `tokio`, `tracing`, `tokio-util` |
| `api.rs` | —/74 | Removed subagent + FTS5 search APIs |
| `forge_api.rs` | —/198 | Removed implementations |

**Merge risk**: **HIGH**. Subagent system and FTS5 search are fork-only.

---

### `crates/forge_app/` — 12 files, ~8 insertions, ~642 deletions

| File | Δ | Analysis |
|------|---|----------|
| `Cargo.toml` | +2/-3 | Version reverted to `0.1.0` |
| `llm_summarizer.rs` | —/253 | **Entire file removed** — fork-only |
| `services.rs` | —/166 | **Entire file removed** |
| `agent.rs` | —/18 | Subagent fields removed |
| `agent_executor.rs` | +1/-11 | Simplified dispatch |
| `tool_registry.rs` | +1/-19 | Simplified registration |
| `orch.rs` | +3/-119 | Subagent orchestration removed |
| `hooks/doom_loop.rs` | —/4 | Subagent hook removed |

**Merge risk**: **HIGH**. Services module and summarizer are fork-only.

---

### `crates/forge_domain/` — 17 files, ~37 insertions, ~2400 deletions

| File | Δ | Analysis |
|------|---|----------|
| `compact/adaptive_eviction.rs` | —/273 | **Entire file removed** |
| `compact/history.rs` | —/172 | **Entire file removed** |
| `compact/importance.rs` | —/328 | **Entire file removed** |
| `compact/metrics.rs` | —/335 | **Entire file removed** |
| `compact/prefilter.rs` | —/319 | **Entire file removed** |
| `compact/strategy.rs` | —/108 | **Entire file removed** |
| `compact/compact_config.rs` | +2/-84 | Stripped compaction fields |
| `compact/mod.rs` | +11/-28 | Stripped to minimum |
| `conversation.rs` | —/73 | **Entire file removed** |
| `intent.rs` | —/133 | **Entire file removed** |
| `repo.rs` | —/246 | **Entire file removed** |
| `telemetry.rs` | —/56 | **Entire file removed** |
| `auth/auth_token_response.rs` | +3/-60 | Removed custom Debug redaction |
| `tools/call/context.rs` | +14/-46 | Removed subagent context fields |

**Merge risk**: **CRITICAL**. Entire `compact/` module (6 files, ~1,535 lines) plus core domain modules deleted upstream.

---

## Complete Repo Diffstat Summary

```
257 files changed, 2013 insertions(+), 25763 deletions(-)
```

The massive net delta is dominated by fork-only additions:
- **Fork-only crates**: `forge3d/`, `forge_drift/`, `forge_dbd/`, `forge_similarity/`
- **Fork-only tooling**: `tooling/forge-context-backfill/`, `tooling/forge-session-cleaner/`, `tooling/forge-vacuum/`
- **Fork-only docs**: `docs/` (SSOT, ADRs, session notes, threat model)
- **Fork-only CI**: `.github/workflows/` (cargo-deny, trufflehog, release-attestation)
- **Fork-only infra**: `shell-plugin/`, `templates/`, `src/`, `deny.toml`, `trufflehog.yml`

---

## Key Divergence Categories

### 1. Fork-Only Feature Additions
- **Subagent conversations** — parent/child trees, `parent_id`, reparenting
- **FTS5 full-text search** — `search_conversations()`, `optimize_fts_index()`
- **LLM summarization** — `llm_summarizer.rs`
- **Advanced compaction pipeline** — importance, metrics, adaptive eviction, prefilter
- **Intent classification** — `intent.rs`
- **Repository abstraction** — `repo.rs` data access trait
- **Telemetry** — `telemetry.rs`
- **Security hardening** — OAuth redaction, cargo-deny, trufflehog, SHA-pinned actions

### 2. Upstream Simplification
- Removed jemalloc, ctrl-c handler, `IsTerminal` check
- Removed `terminal/`, `error.rs`, `model.rs`, `services.rs`

### 3. Fork Branding
- Update URL: `KooshaPari/forgecode`
- Fork-specific CI, docs, version numbers

---

## Merge Risk Assessment

| Component | Risk | Notes |
|-----------|------|-------|
| Dep bumps (11 commits) | **Low** | Safe to cherry-pick |
| Workspace Cargo.toml | **Medium** | Version + dep conflicts |
| forge_main CLI | **High** | 3 deleted modules |
| forge_main UI | **High** | 801-line divergence |
| forge_api | **High** | Subagent/FTS5 removed |
| forge_app orch | **High** | Completely diverged |
| forge_app services | **High** | Entire module removed |
| forge_domain compact/ | **Critical** | 6 files, 1,535 lines |
| forge_domain intent/repo/telemetry | **High** | Entirely deleted |
| Fork-only crates | **Low** | No upstream conflicts |

**Overall**: The 11 dep-bump commits are safe to cherry-pick. Structural divergence is **severe** — upstream removed modules our fork depends on. A full merge would require reimplementing fork features on top of upstream's simplified codebase.

**Recommendation**: Cherry-pick dep-bump commits individually. Do **not** attempt a full merge without a phased refactor plan.

---

## Appendix: Fork Divergence Context

Our fork's `main` contains ~60+ commits not present upstream, including CI hardening, security tooling, fork documentation, the LLM summarizer, compact history fixes, and subagent features. The earliest fork commit branches from merge base `706802e43d`.

