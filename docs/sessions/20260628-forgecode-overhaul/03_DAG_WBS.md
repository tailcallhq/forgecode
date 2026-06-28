# forgecode Overhaul — Phased WBS + DAG

Derived from the 5-cluster deep audit (87 findings). Phases are ordered by dependency; tasks within a phase run in parallel unless a predecessor is listed. Effort is in agent terms (tool calls / parallel subagents / wall-clock), per governance.

## Critical path (one line)
`P0 de-fork docs → P1 CI gates + stub kill + P0 security → P2 resilience/obs/lifecycle → P3 perf+concurrency (needs benches first) → P4 ops/governance docs → P5 cross-repo shared crates (sponsor-gated)`

## DAG (phase predecessors)
```
P0 ─┬─> P1 ─┬─> P2 ─┬─> P3
    │       │       └─> P4
    │       └─> P4
    └────────────────> P5 (also needs sponsor sign-off)
P2 ─> P5
```

---

## Phase P0 — Foundation: de-fork the doc/governance surface  (unblocks scoring + CI)
Predecessors: none. Lowest risk, highest unblock value. Source material already exists in `CLAUDE.md`/`AGENTS.md`/`Cargo.toml`.

| ID | Task | Files | Acceptance | Effort | Dep |
|----|------|-------|-----------|--------|-----|
| P0.1 | Rewrite README Rust-first (remove "ForgeCode Evals TS" framing, real quick-start `cargo build`) | `README.md` | No TS/npm-only claims; `cargo build`/`cargo nextest` documented; arch matches `crates/` | 4 calls / ~3 min | — |
| P0.2 | Rewrite `docs/SSOT.md` to the real 34-crate workspace (kill `Rust: N/A`, fictional ports/adapters) | `docs/SSOT.md` | SSOT lists real crates + layering; no `ProviderPort/CsvAdapter` ghosts | 4 calls / ~3 min | — |
| P0.3 | Replace Node `Justfile` with cargo-driven recipes (`just test`→`cargo nextest`, `just lint`→clippy/fmt) | `Justfile` | `just test`/`just lint`/`just build` drive cargo, exit 0 | 3 calls / ~2 min | — |
| P0.4 | Fill stub governance docs (boundary, intent, journey manifests) | `docs/boundary/forgecode.md`, `docs/journeys/manifests/*` | No `do-not-edit TODO` stubs; real content | 4 calls / ~4 min | — |
| P0.5 | gitignore `.credentials.json` (+ assert 0o600 test still passes) | `.gitignore` | file ignored; regression test green | 1 call / <1 min | — |

**Exit:** docs describe the real product; re-run audit W12/W05/W01 expected ≥+0.7 mean. Wave of 1–2 subagents, ~10–14 calls total.

---

## Phase P1 — Gates & Stubs & Security P0  (after P0 docs give an accurate baseline)
Predecessors: P0 (Justfile/CI docs). Medium risk (clippy may surface debt — fix, don't suppress, per quality policy).

| ID | Task | Files | Acceptance | Effort | Dep |
|----|------|-------|-----------|--------|-----|
| P1.1 | **P0 SECURITY:** redact secrets in `Debug` — wrap `ApiKey`/`AuthCredential`/tokens in a `Secret<String>` or custom `Debug` | `forge_domain` auth types, `provider_repo.rs` | `{:?}` never prints plaintext; test asserts redaction | 5 calls / ~5 min | — |
| P1.2 | Add blocking CI on **Linux runner only** (billing): `cargo fmt --check` + `cargo clippy -D warnings` (replace autofix-only `autofix.yml`) | `.github/workflows/` | PR fails on fmt/clippy violation; not auto-committed | 4 calls / ~4 min | P0.3 |
| P1.3 | Add gating `cargo nextest` job + coverage threshold (stop discarding lcov) | `.github/workflows/`, `forge_ci` | tests gate the merge; threshold enforced | 4 calls / ~4 min | P0.3 |
| P1.4 | Kill production stubs: implement/remove `openai http_delete` `unimplemented!()`; fix `NoopIntentExtractor` erroring | `forge_repo/.../openai_responses/repository.rs#L573`, `forge_domain/src/intent.rs#L119` | no non-test `unimplemented!()`; intent extractor returns or is removed | 5 calls / ~6 min | — |
| P1.5 | Resolve dead/unfinished crates: drop `ghostty-kit`; gate or finish `forge_dbd` | `Cargo.toml`, `crates/ghostty-kit`, `forge_dbd` | workspace has no dead crate; forge_dbd builds+tested or feature-gated | 4 calls / ~5 min | — |
| P1.6 | Collapse update bots to one (kill Renovate blanket `automerge:true`); add `reason` to all advisory ignores | `renovate.json`/`dependabot.yml` | single bot; no unattended automerge; every ignore has a reason+ticket | 3 calls / ~3 min | — |

**Exit:** CI is a real gate; zero production stubs; secret leak closed. L37/L36/L11/L18/L28 lift.

---

## Phase P2 — Hardening: resilience · observability · lifecycle  (after gates green)
Predecessors: P1. Aligns with in-flight branch `fix/5109-proxy-fast-fail-concurrency`.

| ID | Task | Files | Acceptance | Effort | Dep |
|----|------|-------|-----------|--------|-----|
| P2.1 | Unify the 3 divergent backoff impls behind one `RetryConfig`; add circuit breaker + concurrency bulkhead | `mcp_client.rs#L498`, `pool.rs`, central retry | one retry path; breaker trips+recovers (test); bounded concurrency | 8 calls / 2 subagents / ~8 min | P1.2 |
| P2.2 | Metrics facade (`metrics` crate behind a trait) + `tracing` spans on request/exec/stream paths | `forge_*` telemetry | spans cover hot paths; metrics pluggable (noop default) | 8 calls / ~8 min | — |
| P2.3 | `forge_dbd` health probe + graceful drain (don't lose queued writes on exit) | `forge_dbd/src/server.rs#L52` | health endpoint; clean shutdown flushes queue (test) | 6 calls / ~6 min | P1.5 |
| P2.4 | Uniform async task-lifecycle convention; fix uncancellable FTS loop + unbounded forge3d accept loop + fire-and-forget telemetry spawns | `forge_api.rs#L63`, `forge3d/src/server.rs#L225` | long-lived tasks cancellable + bounded; tracked handles | 7 calls / 2 subagents / ~8 min | — |

**Exit:** L5/L26/L4 lift; resilience verifiable.

---

## Phase P3 — Perf & Correctness  (benches MUST exist before optimizing)
Predecessors: P2. Build the measurement spine first, then change allocator/hot paths.

| ID | Task | Files | Acceptance | Effort | Dep |
|----|------|-------|-----------|--------|-----|
| P3.1 | criterion `[[bench]]` spine for 7 hot crates (walker, json_repair, similarity, drift, stream, fs, eventsource) + dhat heap profiling harness | `crates/*/benches` | benches run in CI (non-gating perf job); baseline recorded | 8 calls / 3 subagents / ~10 min | — |
| P3.2 | Swap to jemalloc/mimalloc `#[global_allocator]`; bound unbounded streaming buffers | `forge_main`, `event_stream.rs#L137`, `utf8_stream.rs` | allocator active; buffers capped; bench delta recorded | 5 calls / ~6 min | P3.1 |
| P3.3 | Concurrency verification: loom/miri on the 2 riskiest state machines (MCP client TOCTOU, executor Mutex-across-exec); remove runtime `set_var` (3 files) | `mcp_client.rs#L75`, `executor.rs#L101` | loom/miri job green; no runtime env mutation | 8 calls / 2 subagents / ~12 min | P3.1 |

**Exit:** L6/L8/L7 lift; perf changes are measured, not guessed.

---

## Phase P4 — Ops & Governance docs  (document real, hardened behavior)
Predecessors: P2 (so docs describe actual behavior). Planner-only; no code.

| ID | Task | Acceptance | Effort | Dep |
|----|------|-----------|--------|-----|
| P4.1 | STRIDE threat model (credential store, prompt-injection→subprocess-exec, MCP trust, telemetry egress, ZSH plugin) | `docs/security/threat-model.md` exists, covers all 5 surfaces | 1 subagent / ~12 min | P2.1 |
| P4.2 | Ops doc set: SLO/error-budget (CLI-appropriate), runbook, incident/postmortem template | `docs/operations/*` complete | ~8 calls / ~8 min | P2.3 |

**Exit:** L27/L20 off the floor (0.3/0.5 → ≥1.8).

---

## Phase P5 — Cross-repo shared crates  (SPONSOR-GATED)
Predecessors: P0 + P2 + **explicit sponsor sign-off** on destination per the Phenotype Cross-Project Reuse Protocol. ~3.5–5.5k LOC duplicated with OmniRoute & cliproxyapi-plusplus.

| ID | Task | Acceptance | Effort | Dep |
|----|------|-----------|--------|-----|
| P5.1 | Extract `phenotype-provider-models` (provider/model registry + schema normalization) | new shared crate; forgecode+OmniRoute+cliproxy consume it; dup removed | 3 subagents / ~20 min | sponsor sign-off |
| P5.2 | Extract shared OAuth2 (+5-min refresh buffer) | shared crate; callers migrated | 2 subagents / ~15 min | P5.1 |
| P5.3 | Extract shared resilience/SSE stop-signal | shared crate; callers migrated | 2 subagents / ~15 min | P5.1, P2.1 |

**Exit:** L35 → ≥2.6; org-wide dedup.

---

## Execution waves (recommended)
- **Wave 1 (now, parallel):** all of P0 (1–2 subagents) + P1.1 (security P0) + P1.4/P1.5 (stubs) — independent, ~15 min.
- **Wave 2:** P1.2/P1.3/P1.6 (CI gates) after P0.3 — ~10 min.
- **Wave 3:** P2 (4 tasks parallel) — ~10 min.
- **Wave 4:** P3.1 then P3.2/P3.3; P4 in parallel — ~15 min.
- **Wave 5:** P5 after sponsor sign-off.

## Projected pillar lift
Weak-cluster means **0.83–1.40 → ~2.0–2.4** after P0–P4. Each phase ends by re-running the relevant v37 cluster audit to confirm the lift (smart-contract verification).

## Sponsor decisions required
1. **P5 destination** for shared crates (new repo vs existing shared module) — per reuse protocol.
2. **Scope/order:** ship P0+P1 as the first PR train, or batch P0–P2?
3. **Phase B audit:** deep-audit the mid-tier clusters (W06/W08 at 1.50) too, or focus the overhaul on the bottom 5?
