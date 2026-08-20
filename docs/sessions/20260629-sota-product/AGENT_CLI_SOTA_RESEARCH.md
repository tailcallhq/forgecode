# forgecode — SOTA Agentic Coding CLI Research Dossier

**Date:** 2026-06-29
**Scope:** Push forgecode (Rust agentic coding CLI/TUI) to state-of-the-art as a product.
**Type:** Planner / research dossier. No forgecode source changes proposed inline — references and acceptance criteria only.
**Method:** First-hand competitor web research (WebSearch/WebFetch), arxiv/technical research, and direct reading of forgecode's crates (`forge_domain`, `forge_app`, `forge_services`, `forge_repo`, `forge_main`, …).

---

## 0. forgecode — Grounded Baseline (from the code)

Read directly from the repo at `crates/`:

- **Architecture:** Hexagonal, 33-crate Cargo workspace. Pure domain (`forge_domain`), composition root (`forge_app`), orchestration (`forge_services`), public async-trait API (`forge_api`), infra adapters (`forge_infra`), provider/persistence (`forge_repo`), binary (`forge_main`). Clean ports-and-adapters separation — a genuine architectural strength vs. most competitors.
- **Agent loop:** `forge_app/src/orch.rs` — `Orchestrator<S>` with `#[async_recursion]`, tool-error tracker (`ToolErrorTracker`), hooks (`Arc<Hook>`), pluggable `MetricsSink`. Real multi-step tool loop.
- **Agents (multi-persona):** 3 built-in — `forge`, `muse`, `sage` (`forge_domain/src/agent.rs`; defs in `forge_repo/src/agents/{forge,muse,sage}.md`). `AgentId` constants. Reasoning config (effort levels None→High, max_tokens, exclude/enabled).
- **Tools:** `forge_services/src/tool_services/` — `fs_read`, `fs_write`, `fs_patch`, `fs_remove`, `fs_search`, `fs_undo`, `shell`, `fetch`, `followup`, `plan_create`, `skill`, `image_read`, plus `code_review` (in tool catalog). Tool catalog at `forge_domain/src/tools/catalog.rs`.
- **MCP:** Full client + manager (`forge_services/src/mcp/{manager,service,tool}.rs`, `forge_infra/src/mcp_client.rs`, `forge_domain/src/mcp_servers.rs`). Tools surface alongside built-ins.
- **Context engineering:** `forge_domain/src/compact/` — `adaptive_eviction`, `importance`, `prefilter`, `strategy`, `summary`, `metrics`, `history`. This is a real, sophisticated compaction subsystem (above-average vs. peers).
- **Skills:** `forge_domain/src/skill.rs` + `forge_repo/src/skills/` — markdown skills with resources (Claude-Code-style).
- **Sessions/memory:** SQLite session store w/ WAL checkpointing + zstd compression, conversation FTS + vector search, subagent breadcrumbs (README; `forge_dbd` session daemon over Unix socket; `forge_embed` embeddings — but `forge_embed/src/hash_only.rs` indicates a **hash-only / non-semantic** embed path).
- **Providers/routing:** `forge_repo/src/provider/` — Anthropic, OpenAI, OpenAI Responses, Google, Bedrock (+cache/sanitize), OpenCode, OpenRouter-style. Multi-provider with retry. `model_config.rs`, `agent_provider_resolver.rs`.
- **Safety:** `forge_main/src/sandbox.rs` (sandbox), policies (`forge_domain/src/policies`, `forge_services/src/permissions.default.yaml`), hooks.
- **TUI/render:** `forge_tui`, `forge_display`, `forge_spinner`, `forge_select`, `forge_markdown_stream`, `ghostty-kit`, `forge3d` (visualization).
- **Version:** workspace `2.9.9` (README claims 2.10.0 — minor drift). ~280 files contain tests.

**What the code does NOT contain (confirmed by targeted grep — no real hits):**
- ❌ No LSP / language-server integration (no rust-analyzer/pyright/gopls wiring).
- ❌ No AST/tree-sitter repo map or PageRank symbol-graph context selection (the `tool_services/syn/` dir is a stub: only `mod.rs`).
- ❌ No browser automation tool.
- ❌ No true parallel multi-session / git-worktree agent orchestration (sandbox exists; parallel agents do not).
- ❌ No lint/test autofix loop (no "run tests → repair → retry" harness).
- ❌ No semantic embeddings in the default path (hash-only).
- ❌ No first-class cost/budget UI surface (telemetry exists; per-session $ ceiling does not).

---

## 1. Competitor Capability Matrix

Legend: ✅ yes · ⚠️ partial/weak · ❌ no · **FC** = forgecode (from code above).

| Capability | Claude Code | Codex CLI | Cursor CLI | Aider | Gemini CLI | OpenCode | Cline | Goose | Continue | **forgecode** |
|---|---|---|---|---|---|---|---|---|---|---|
| Agent tool loop | ✅ | ✅ | ✅ | ⚠️ (pair) | ✅ | ✅ | ✅ | ✅ | ⚠️ | ✅ |
| MCP support | ✅ | ✅ | ✅ | ⚠️ | ✅ | ✅ | ✅ | ✅ (native) | ✅ | ✅ |
| Multi-provider/model routing | ⚠️ (Claude) | ⚠️ (OpenAI) | ⚠️ | ✅ (100+) | ⚠️ (Gemini) | ✅ (75+) | ✅ | ✅ | ✅ | ✅ |
| Sessions persist/resume | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ (+share links) | ⚠️ | ✅ | ✅ | ✅ (SQLite+FTS+vec) |
| Long-term memory | ⚠️ | ⚠️ | ⚠️ | ⚠️ (repo map) | ⚠️ | ⚠️ | ✅ (memory bank via Kilo)| ⚠️ | ⚠️ | ⚠️ (FTS/vec, hash-only) |
| Subagents / multi-agent | ✅ | ✅ | ⚠️ | ❌ | ❌ | ✅ | ❌ | ✅ (modes) | ❌ | ✅ (3 agents+breadcrumbs) |
| Parallel multi-session | ⚠️ | ⚠️ | ✅ | ❌ | ❌ | ✅ | ❌ | ⚠️ | ❌ | ❌ |
| Multi-file edits | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| LSP integration | ❌ | ❌ | ✅ | ⚠️ | ❌ | ✅ (18+ langs) | ❌ | ❌ | ✅ | ❌ |
| AST/repo-map context | ⚠️ | ⚠️ | ✅ | ✅ (tree-sitter+PageRank, 130+ langs) | ⚠️ | ✅ | ⚠️ | ✅ (indexing) | ✅ | ❌ |
| Test generation | ⚠️ | ⚠️ | ⚠️ | ✅ (auto test+lint) | ⚠️ | ⚠️ | ⚠️ | ✅ | ⚠️ | ❌ |
| Lint/test autofix loop | ⚠️ | ⚠️ | ⚠️ | ✅ | ⚠️ | ⚠️ | ⚠️ | ✅ | ⚠️ | ❌ |
| Code review | ✅ (PR) | ✅ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ | ⚠️ (tool only) |
| Checkpoints / undo | ✅ | ✅ | ✅ | ✅ (git) | ✅ | ✅ | ✅ | ✅ | ⚠️ | ⚠️ (fs_undo only) |
| Approval/sandbox modes | ✅ | ✅ (3 modes, OS sandbox) | ✅ | ⚠️ | ⚠️ | ✅ | ✅ (review-first) | ✅ | ✅ | ⚠️ (sandbox+policy) |
| Cost control / budget UI | ⚠️ | ⚠️ | ⚠️ | ✅ (you pay provider) | ✅ (free tier) | ✅ (free models) | ✅ | ✅ | ✅ | ❌ |
| Local/offline (Ollama) | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ | ✅ | ✅ | ✅ | ⚠️ (OpenAI-compat) |
| Browser automation | ⚠️ | ❌ | ⚠️ | ❌ | ⚠️ | ⚠️ | ✅ (screenshots) | ⚠️ | ❌ | ❌ |
| Hooks / extensibility | ✅ | ✅ | ⚠️ | ⚠️ | ⚠️ | ✅ | ✅ (MCP) | ✅ | ✅ | ✅ (hooks) |
| Rich TUI UX | ✅ | ✅ | ⚠️ (IDE) | ⚠️ | ✅ | ✅ | ✅ (VS Code) | ✅ | ✅ (IDE) | ✅ (TUI+ghostty+3d) |
| Skills | ✅ | ⚠️ | ❌ | ❌ | ❌ | ⚠️ | ❌ | ⚠️ (modes) | ❌ | ✅ |

**Net read:** forgecode is strong on architecture, MCP, multi-agent, skills, context compaction, and session persistence. It is materially **behind SOTA on code-comprehension infrastructure (LSP + AST repo map), the test/repair loop, parallel multi-session orchestration, cost/budget surfacing, and semantic memory.** Those are precisely the dimensions that move SWE-bench-style real-world performance and developer trust.

Sources: [Tembo 2026 CLI comparison](https://www.tembo.io/blog/coding-cli-tools-comparison) · [Claude Code 2026 features (MarkTechPost)](https://www.marktechpost.com/2026/06/14/claude-code-guide-2026-25-features-with-examples-demo/) · [Codex CLI sandbox/approvals](https://developers.openai.com/codex/concepts/sandboxing) · [Codex subagents](https://developers.openai.com/codex/subagents) · [OpenCode LSP docs](https://opencode.ai/docs/lsp/) · [Aider repo map](https://aider.chat/docs/repomap.html) · [Goose / Cline / Gemini CLI roundup (jock.pl)](https://thoughts.jock.pl/p/ai-coding-harness-agents-2026) · [Morph LLM agent ranking](https://www.morphllm.com/ai-coding-agent)

---

## 2. Technical Research — Adoptable Techniques (cited)

### 2.1 Code comprehension & context selection
- **Tree-sitter + PageRank repo map (Aider).** AST symbol extraction → file dependency graph → PageRank to rank identifiers → fit top-ranked context into a token budget; 130+ languages via `tags.scm` queries; battle-tested at 15B tokens/week. **Highest-leverage single adoptable technique for forgecode.** Source: [aider repomap](https://aider.chat/2023/10/22/repomap.html), [aider docs](https://aider.chat/docs/repomap.html).
- **LSP-as-context (OpenCode).** Feed the model real type info, signatures, import paths, and live compiler diagnostics (rust-analyzer/pyright/gopls/clangd…); auto-load LSP per language. Source: [OpenCode LSP](https://opencode.ai/docs/lsp/).
- **AST-guided adaptive memory (CodeMEM).** AST-structured memory for repository-level iterative code generation. Source: [arXiv 2601.02868](https://arxiv.org/pdf/2601.02868).

### 2.2 Context management & memory (forgecode already has a compaction base to extend)
- **Meta Context Engineering** treats context assembly as an optimization problem: **89.1% SWE-bench Verified vs 70.7%** for hand-engineered baselines — context assembly, not raw model, is the dominant lever. Source: [arXiv 2603.07670](https://arxiv.org/html/2603.07670v1).
- **MemGPT-style memory-as-decision** (LLM decides when to retrieve/manage long-term context) and **GraphRAG / RAPTOR / Self-RAG** retrieval. Source: [memory survey arXiv 2603.07670](https://arxiv.org/html/2603.07670v1).
- **Confucius Code Agent** — explicit agent context layer: hierarchical working memory + adaptive compression + filesystem-backed scoped visibility. Direct analog to extend `forge_domain/src/compact/`. Source: [arXiv 2512.10398](https://arxiv.org/pdf/2512.10398).
- **AgentOCR** — optical self-compression of agent history (compress trajectory rather than drop it). Source: [arXiv 2601.04786](https://arxiv.org/pdf/2601.04786).
- **Structured memory that grows with the user** for code agents. Source: [arXiv 2603.13258](https://arxiv.org/pdf/2603.13258).

### 2.3 Repair & evaluation scaffolds
- **Agentless pipeline** (localize → repair → validate, one-shot; +test-gen-by-considering-other-tests +patch-selection-by-voting) is competitive with full agents and cheap. Adopt voting/patch-selection as a forgecode "repair mode." Source: [SWE-bench leaderboard dissection arXiv 2506.17208](https://arxiv.org/html/2506.17208v2), [OpenAI SWE-bench Verified](https://openai.com/index/introducing-swe-bench-verified/).
- **Scaffold matters more than the base model** — retrieval, tools, recovery-from-failed-patches, retries change scores materially. SWE-agent (agent–computer interface), OpenHands (multi-agent), AutoCodeRover (AST search). Source: [arXiv 2506.17208](https://arxiv.org/html/2506.17208v2).
- **Long-horizon eval beyond single-issue SWE-bench:** SWE-EVO (software evolution), LoCoBench-Agent (long-context SWE), AMA-Bench (long-horizon memory). Adopt as forgecode's own internal eval harness. Sources: [SWE-EVO 2512.18470](https://arxiv.org/pdf/2512.18470), [LoCoBench-Agent 2511.13998](https://arxiv.org/pdf/2511.13998), [AMA-Bench 2602.22769](https://arxiv.org/pdf/2602.22769).

### 2.4 Serving / efficiency
- **KV-cache sharing across LLMs (DroidSpeak)** — relevant for forgecode's multi-provider routing + future multi-agent fan-out. Source: [arXiv 2411.02820](https://arxiv.org/pdf/2411.02820).
- **MemSearcher** — RL-trained reason+search+manage-memory. Source: [arXiv 2511.02805](https://arxiv.org/pdf/2511.02805).

---

## 3. User / Market Needs (cited)

1. **Verification bottleneck is the #1 pain.** Devs report **~11.4 hrs/week reviewing AI code vs 9.8 hrs writing** new code — review/test tooling now matters more than raw generation. Source: [Faros AI](https://www.faros.ai/blog/best-ai-coding-agents-2026), [Sonar State of Code 2026](https://www.sonarsource.com/state-of-code-developer-survey-report.pdf).
2. **Harness-level reliability > model IQ.** Claude Code quality regressions (default-effort changes, reasoning-history bugs) eroded trust — proves operational reliability is a product axis. Source: [Faros AI](https://www.faros.ai/blog/best-ai-coding-agents-2026).
3. **Cost transparency & control.** Cursor pricing backlash is a recurring community theme; pay-as-you-go-with-no-markup and free local models are explicit selling points (Kilo, OpenCode). Source: [Faros AI](https://www.faros.ai/blog/best-ai-coding-agents-2026), [Tembo](https://www.tembo.io/blog/coding-cli-tools-comparison).
4. **Autonomy WITH control (review-first).** Cline's "conservative, review-first" workflow is praised precisely because it fits existing practice. Approval/sandbox modes (Codex) are table stakes. Source: [Faros AI](https://www.faros.ai/blog/best-ai-coding-agents-2026), [Codex approvals](https://developers.openai.com/codex/agent-approvals-security).
5. **Privacy / data control & model-agnosticism.** Open-source + BYO-model + local/offline are differentiators teams actively want. Source: [Open Source AI Review](https://www.opensourceaireview.com/blog/best-open-source-ai-coding-agents-in-2026-ranked-by-developers).
6. **Parallelism / running two agents.** "Top devs run two" — parallel sessions and orchestration are emerging expectations. Source: [AI Builder Club](https://www.aibuilderclub.com/blog/best-ai-coding-agent-2026), [OpenCode multi-session](https://opencode.ai/docs/lsp/).

---

## 4. Gap Analysis (code-grounded)

| # | Gap | Evidence in forgecode | SOTA reference | Impact |
|---|---|---|---|---|
| G1 | **No code-aware context (LSP + AST repo map)** | `tool_services/syn/` is a `mod.rs` stub; no tree-sitter/PageRank/LSP deps | Aider repo map, OpenCode LSP | HIGH — directly caps real-codebase accuracy |
| G2 | **No test/lint autofix repair loop** | no run-tests→repair harness in `forge_app` | Aider auto test+lint; Agentless validate | HIGH — verification bottleneck is #1 user pain |
| G3 | **No parallel multi-session / worktree orchestration** | `sandbox.rs` only; single conversation in `Orchestrator` | OpenCode, Cursor, "run two" | MED-HIGH — emerging expectation |
| G4 | **Hash-only embeddings (no semantic memory)** | `forge_embed/src/hash_only.rs` | MemGPT, GraphRAG, structured memory | MED-HIGH — FTS/vec store underdelivers |
| G5 | **No cost/budget surface** | telemetry exists; no per-session $ ceiling/UI | Kiro credits, Kilo PAYG, Gemini free-tier UX | MED — trust + adoption |
| G6 | **Code review is a tool, not a workflow** | `code_review` in catalog; no PR/diff review agent loop | Amp review agent, Claude Code PR | MED — addresses verification pain |
| G7 | **Approval modes under-productized** | sandbox + `permissions.default.yaml` exist; no clear read-only/auto/full UX | Codex 3-mode approvals | MED — table-stakes safety UX |
| G8 | **No internal SWE-style eval harness** | `benchmarks/` exists but no SWE-bench/long-horizon loop | SWE-EVO, LoCoBench-Agent | MED — regression-proofs reliability (need #2) |
| G9 | **Context engine not optimization-driven** | `compact/` is heuristic (importance/eviction) | Meta Context Engineering 89.1% vs 70.7% | MED — biggest measured single lever |

### Top-5 SOTA gaps
1. **G1 — Code-aware context (Tree-sitter repo map + LSP diagnostics).**
2. **G2 — Test/lint autofix repair loop (validate→repair→retry).**
3. **G4/G9 — Semantic memory + optimization-driven context engine** (replace hash-only embeds; treat context as optimization).
4. **G3 — Parallel multi-session / worktree orchestration.**
5. **G5/G6/G7 — Trust trifecta: cost/budget UI + review workflow + first-class approval modes.**

---

## 5. Prioritized SOTA Roadmap (DAG + effort + acceptance + test coverage)

**Coverage mandate (every feature):** 85–100% across **unit → e2e → perf → chaos**. Concretely: unit tests on pure logic (`forge_domain`); integration/e2e through `forge_api`/`forge_main`; perf benchmarks in `benchmarks/` (token budget, latency, ranking); chaos (provider timeouts, malformed tool output via `forge_json_repair`, LSP crash, cancelled sessions, partial patch failure).

### DAG (dependencies)
```
P0 ── F1 (Tree-sitter repo map) ──┐
       │                          ├── F4 (Repair loop) ── F8 (SWE eval harness)
       └── F2 (LSP diagnostics) ──┘                          ▲
P0 ── F3 (Semantic memory) ── F9 (Context-as-optimization) ──┘
P1 ── F5 (Approval modes) ── F6 (Review workflow)
P1 ── F7 (Cost/budget UI)
P2 ── F10 (Parallel multi-session / worktrees)  [depends F1,F4]
P2 ── F11 (Local/offline + Ollama first-class)  [independent]
```

### Phase 0 — Comprehension foundation (unblocks everything)

**F1 — Tree-sitter AST repo map + PageRank context selection** *(predecessor: none)*
- Effort: major refactor — 3–5 parallel subagents, ~15–20 min batches. Lands in `forge_services` (new `repo_map` service) + `forge_domain` ranking types; wire `tags.scm` per language; flesh out `tool_services/syn/`.
- Acceptance: given a repo + query, returns ranked symbol context fitting a configurable token budget; ≥100 languages via tree-sitter; deterministic ranking; measurable accuracy lift on internal eval (F8).
- Coverage: unit (parse/rank/budget-fit) ≥95%; e2e (real repos) ; perf (rank+assemble < target ms on 10k-file repo); chaos (unparseable files, symlink loops, binary files).

**F2 — LSP diagnostics-as-context** *(predecessor: none; complements F1)*
- Effort: cross-stack — 2–3 subagents, ~8 min. New `forge_infra` LSP adapter (rust-analyzer/pyright/gopls/clangd), auto-load per language; expose diagnostics+signatures to orchestrator.
- Acceptance: edits validated against live diagnostics; signature/import info injected pre-edit; graceful when no LSP present.
- Coverage: unit (protocol codec) ; e2e (per-language smoke) ; perf (LSP startup amortized) ; chaos (LSP crash/restart, slow server timeout → degrade not hang).

**F3 — Semantic memory (replace hash-only embeddings)** *(predecessor: none)*
- Effort: cross-stack — ~8 min. Real embedding provider in `forge_embed` behind existing trait; populate vector search; MemGPT-style retrieve-on-demand over `forge_dbd` store.
- Acceptance: semantic recall beats FTS baseline on a held-out conversation-recall set; configurable local/remote embedder; no PII leakage.
- Coverage: unit (embed/index/query) ≥90%; e2e (recall@k) ; perf (index/query latency) ; chaos (embedder outage → fall back to FTS visibly, not silently).

### Phase 1 — Trust & verification

**F4 — Validate→repair→retry loop** *(predecessors: F1, F2)*
- Effort: major — 3–5 subagents, ~15 min. New harness in `forge_app`: after edits, run lint+tests, parse failures, repair, bounded retries; Agentless-style patch voting/selection.
- Acceptance: on a seeded broken-test suite, autonomously reaches green within N retries; never loops infinitely; emits a diff+rationale.
- Coverage: unit (failure parsing, retry bound, voting) ; e2e (red→green on fixture repos) ; perf (retry budget) ; chaos (flaky tests, non-deterministic failures, partial patch apply).

**F5 — First-class approval/sandbox modes** *(predecessor: none)*
- Effort: small-feature — ~3 min. Productize `permissions.default.yaml`+`sandbox.rs` into read-only / auto / full modes with in-session `/permissions` switching (Codex parity).
- Acceptance: mode gates tool execution correctly; switchable mid-session; audit log of approvals.
- Coverage: unit (gate matrix) ≥95%; e2e (mode transitions) ; chaos (privilege-escalation attempts blocked).

**F6 — Code review workflow (diff/PR review agent)** *(predecessor: F5)*
- Effort: cross-stack — ~8 min. Promote `code_review` tool to a review agent loop over diffs/PRs with severity-ranked findings.
- Acceptance: produces actionable, ranked review on a diff; integrates with git; suppresses noise.
- Coverage: unit (finding ranking) ; e2e (real diffs) ; perf (large-diff handling) ; chaos (binary/huge diffs).

**F7 — Cost/budget surface** *(predecessor: none)*
- Effort: small-feature — ~3 min. Per-session token/$ tracking in TUI from existing telemetry; configurable ceiling that pauses for approval.
- Acceptance: live cost shown; ceiling pause works; per-provider pricing table.
- Coverage: unit (cost math per provider) ≥95%; e2e (ceiling pause) ; chaos (missing pricing → conservative estimate, flagged).

### Phase 2 — Scale & reach

**F8 — Internal SWE/long-horizon eval harness** *(predecessors: F1, F4)*
- Effort: major — ~15 min. SWE-bench-Verified-style + SWE-EVO/LoCoBench-style runner in `benchmarks/`; CI-gated regression scores.
- Acceptance: reproducible pass-rate report; blocks merges on regression; tracks cost/latency per task.
- Coverage: e2e (harness runs) ; perf (throughput) ; chaos (sandbox isolation, timeouts).

**F9 — Context-as-optimization engine** *(predecessors: F1, F3)*
- Effort: major — ~15 min. Evolve `forge_domain/src/compact/` from heuristic to optimization-driven assembly (Meta Context Engineering direction).
- Acceptance: measurable lift on F8 vs current heuristic compaction; bounded assembly latency.
- Coverage: unit (assembler) ; e2e (eval lift) ; perf (assembly budget) ; chaos (pathological histories).

**F10 — Parallel multi-session / git-worktree orchestration** *(predecessors: F1, F4)*
- Effort: major — ~20 min. Multiple concurrent agent sessions over isolated worktrees; merge/coordination.
- Acceptance: N parallel sessions on one repo without state corruption; isolated worktrees; clean merge path.
- Coverage: unit (session isolation) ; e2e (2+ parallel) ; perf (concurrency cap) ; chaos (worktree lock contention, conflicting edits, crash mid-session).

**F11 — Local/offline first-class (Ollama)** *(predecessor: none)*
- Effort: small-feature — ~3 min. Promote OpenAI-compatible local endpoints to a documented Ollama profile.
- Acceptance: fully local run works offline; model list discovery.
- Coverage: unit (endpoint cfg) ; e2e (offline smoke) ; chaos (endpoint down → clear error, no silent remote fallback).

---

## 6. Quick-win sequencing (aggressive, agent-driven)
- **Wave 1 (parallel):** F1, F2, F3, F5, F7 — 5 subagents, comprehension + trust basics.
- **Wave 2:** F4, F6 — repair loop + review (need F1/F2/F5).
- **Wave 3:** F8, F9, F10, F11 — eval, optimized context, parallelism, local.

This ordering front-loads the two highest-leverage SOTA gaps (code-aware context, repair loop) and the verification/trust features users most want, while the eval harness (F8) locks in reliability — the exact axis where competitors (Claude Code regressions) lost user trust.

---

## 7. Citations (consolidated)
Competitors: Tembo CLI comparison; MarkTechPost Claude Code 2026; Codex sandboxing/approvals/subagents (developers.openai.com); OpenCode LSP docs; Aider repomap (2 pages); jock.pl harness comparison; Morph LLM ranking; Faros AI reviews; Sonar State of Code 2026; Open Source AI Review; AI Builder Club; Developers Digest Claude Code teams; callsphere agent loop; code.claude.com Agent SDK.
Technical (arXiv): 2603.07670 (memory survey + Meta Context Engineering); 2512.10398 (Confucius); 2601.02868 (CodeMEM); 2601.04786 (AgentOCR); 2603.13258 (growing memory); 2506.17208 (SWE-bench dissection / Agentless); 2512.18470 (SWE-EVO); 2511.13998 (LoCoBench-Agent); 2602.22769 (AMA-Bench); 2411.02820 (DroidSpeak); 2511.02805 (MemSearcher); openai.com SWE-bench Verified; epoch.ai SWE-bench Verified.

**Total distinct cited sources: ~30** (16 competitor/market URLs + 14 arXiv/technical/eval references).
