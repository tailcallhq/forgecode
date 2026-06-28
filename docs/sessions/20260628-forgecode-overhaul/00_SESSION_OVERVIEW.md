# forgecode Overhaul — Session Overview

**Date:** 2026-06-28 · **Owner:** orchestrated (parent coordinator + audit fleet) · **Repo:** forgecode (34-crate Rust workspace, fork of antinomyhq/forge; powers `forge-dev`)

## Goal
Own a deep, evidence-based audit of forgecode against the L0–L40 pillar framework (v37) and produce a phased, DAG-structured overhaul roadmap that lifts the weakest pillars to ≥2.0 without regressing the strong ones.

## Method
Phase 1 — deep-audit fleet (5 agents, one per weakest cluster) auditing **current canonical `main`** (not the stale audit clone), each producing file-cited findings with target state, work items, acceptance criteria, agent-effort, dependencies, and risk. Phase 2 — this synthesis: a phased WBS + DAG.

## Scorecard baseline (v37 means; lower = higher leverage)
| Cluster | Pillars | Mean | Theme |
|---|---|---|---|
| W03 | L5,L26,L27 | **0.83** | observability · resilience · failure-ops |
| W02 | L4,L6,L7,L8 | **1.00** | async lifecycle · perf · concurrency · memory |
| W12 | L34–L37 | **1.15** | docs · shared-code · polish · stubs |
| W07 | L18,L19,L20,L28 | **1.38** | secrets · supply-chain · threat-model · deps |
| W05 | L11,L12,L13 | **1.40** | testing-DX · SSOT docs · onboarding |
| W06 | L14–L17 | 1.50 | (not deep-audited — Phase B) |
| W08 | L21–L24,L29 | 1.50 | (Phase B) |
| W10/W09/W04/W11/W01 | — | 1.75–2.23 | already strong |

## Root cause (cross-cutting)
The single highest-leverage finding, surfaced independently by **3 of 5** audits (W12, W05, and the W01 re-score): the repo still carries **leftover "ForgeCode Evals" TypeScript fork scaffolding** (README, `docs/SSOT.md`, `Justfile`, boundary/intent stub docs) that describes a fictional TS project, while the real product is a ~144k-LOC Rust workspace. This drift causes doc-pillar failures, ungated CI (Justfile drives Node), audit misscoring, and stub-detection penalties. **De-forking the doc/governance surface is the foundational unblocker** for the whole roadmap.

## Deliverables
- `audit/` — 5 cluster findings docs (87 findings total, all file-cited).
- `03_DAG_WBS.md` — the phased overhaul roadmap (this session's primary output).
- `05_KNOWN_ISSUES.md` — live bugs found during audit (P0 secret leak, production `unimplemented!()`).

## Live bugs found (not just scores)
- **P0 secret leak:** `#[derive(Debug)]` on `ApiKey`/`AuthCredential`/OAuth tokens prints plaintext to logs/PostHog (only `Display` redacts).
- **P1:** `.credentials.json` (0o600) not gitignored.
- **P1:** production `unimplemented!()` in `forge_repo/.../openai_responses/repository.rs#L573` (`http_delete`).
- **P2:** user-facing `panic!` on bad `--directory` (`forge_main/src/main.rs#L135`).
