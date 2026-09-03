# SOTA Agentic Coding CLIs/TUIs — Competitor Capability Dossier (Mid-2026)

> Research input for `AGENT_CLI_SOTA_RESEARCH.md`. Evidence base: 4 parallel research streams against official docs, changelogs, GitHub, and reputable analyses. Claims sourced in the citations block; unconfirmed features marked partial/unverified.

## 1. Capability Matrix

Legend: ✓ supported/strong · ◐ partial/limited/unverified · ✗ absent · n/d no data

| Capability | Claude Code | Codex CLI | Cursor CLI | Aider | Gemini/Antigravity | OpenCode | Continue | Cline | Goose | Amp | Crush | Warp | Factory Droid |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Multi-file edit | ✓ | ✓ | ✓ worktrees | ✓ 100+ langs | ✓ 1M ctx | ✓ | ✓ | ✓ | ✓ | ✓ index | ✓ | ✓ | ✓ |
| MCP | ✓ client+server | ✓ | ✓ | ◐ | ✓ native | ✓ OAuth | ◐ | ✓ marketplace | ✓ 70+ | ◐ | ✓ stdio/HTTP/SSE | ✓ | ◐ |
| Subagents/multi-agent | ✓ Agent SDK | ✓ max_depth | ✓ /multitask | ✗ | ◐ | ✗ | ◐ | ✓ Kanban | ◐ | ✓ specialist | ◐ | ✓ Cloud | ✓ Droids |
| Plan mode | ✓ | ✓ | ✓ | ✓ architect | ✓ | ✓ | ◐ | ✓ Plan/Act | ◐ | ✓ | ◐ | ✓ | ✓ |
| Sandboxing | ✓ -84% prompts | ✓ container | ✓ classifier | ✗ git | ✗ | ✗ | ✗ | ◐ | ◐ | ◐ | ◐ yolo | ✓ Cloud | ◐ |
| Model routing | ◐ single+effort | ✓ 3-tier | ◐ manual | ✓ arch/editor | ◐ single | ✓ BYOK 8+ | ✓ 40+ | ✓ 6+ | ✓ 15+ | ✓ auto | ✓ mid-session | ✓ | ✓ per-Droid |
| Session memory/compaction | ✓ hier+auto | ◐ | ✓ rules+resume | ◐ repo-map | ✓ checkpoint | ✓ sophisticated | ✓ | ◐ | ◐ | ◐ | ◐ | ✓ Drive | ◐ |
| Checkpoints/undo | ✓ /rewind | ◐ | ✓ | ✓ git | ✓ shadow repo | ✓ undo/redo | ◐ | ✓ per-step | ◐ | ◐ | ◐ | ◐ | ◐ |
| Test-gen/verify loop | ✓ | ✓ browser | ✓ 8-pass | ✓ test-repair | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ◐ | ◐ | ✓ |
| Headless/CI | ✓ | ✓ exec JSONL | ✓ --print | ✓ | ✓ | ✓ | ✓ -p JSON | ✓ | ✓ | ✓ | ◐ | ✓ webhook | ✓ |
| TUI quality | ✓ | ✓ | ✓ vim | ◐ | ◐ | ✓ Ink | ✓ Ink | ✓ Kanban | ✓ | CLI | ✓✓ glam | ✓✓ | ◐ |
| Local/offline | ◐ | ✗ | ✗ | ✓ Ollama 50+ | ◐ | ✓ BYOK | ✓ | ✓ | ✓ | ✗ | ✓ | ✓ | ◐ |
| Cost controls | ◐ | ◐ | ◐ | ✓ cheap editor | ✓ caching | ◐ | ◐ | ✓ spend limits | ◐ | ◐ | ◐ | ◐ | ✓ effort |
| Background/async | ✓ | ✓ | ✓ Cloud VMs | ◐ watch | ◐ | ✗ | ◐ | ✓ cron | ◐ | ◐ | ✗ | ✓ Oz | ◐ |

## 2. The 2026 SOTA frontier — 15 highest-leverage capabilities
1. Loop-engineering agent core (gather→act→verify + autonomous test/lint/repair).
2. Multi-tier model routing (frontier planner + cheap fast executor).
3. Parallel multi-agent over isolated git worktrees with dependency chains.
4. OS-level sandboxing that minimizes approval fatigue (the trust differentiator).
5. Sophisticated context compaction (threshold summary + message hiding + cushion).
6. Hierarchical persistent memory (rules files + auto-memory + git-as-memory).
7. Cloud/background async agents (isolated VM, webhook/cron, PR-on-done, audit logs).
8. First-class MCP (client+server; stdio/HTTP/SSE; OAuth) — table stakes.
9. Explicit plan mode with approval gating before mutation.
10. Checkpoints & rewind/undo of conversation AND file state.
11. Clean headless/CI mode (no-TTY, JSON/JSONL streaming, pipeable).
12. Lifecycle hooks (Pre/PostToolUse, policy gates, scheduled agents).
13. Whole-repo context strategy (1M–2M windows and/or efficient repo-maps/indexes).
14. Provider-agnostic BYOK incl. local/offline + explicit cost/spend controls.
15. High-craft TUI/UX (mode switch, vim, live tool/reasoning visibility, glamorous render).

**Meta-trend: agent design beats raw model choice** — Factory Droid is #1 on Terminal-Bench (58.75%) with sub-frontier models, beating Claude Code & Codex. Battleground shifted from prompt-engineering to loop-engineering + permission/trust UX + multi-agent orchestration.

## 3. Status flags (avoid anchoring on tools mid-transition)
- **Gemini CLI** sunset for free/individual tiers 2026-06-18 → successor **Antigravity CLI** (early, gaps).
- **OpenCode** archived (Sept 2025) → dev moved to **Charm Crush**.
- **Continue CLI** frozen at v2.0 after Cursor acquisition (June 2026).
- **Cursor** sandbox: CVE-2026-22708 (bypass) — note for security comparisons.

## 4. Citations
Anthropic/OpenAI: anthropic.com/product/claude-code, code.claude.com/docs, anthropic.com/engineering/claude-code-sandboxing, developers.openai.com/codex, github.com/openai/codex, thenewstack.io/loop-engineering. Cursor/Aider/Gemini: cursor.com/docs/cli, cursor.com/docs/cloud-agent, aider.chat/docs, github.com/aider-ai/aider, developers.google.com/gemini-code-assist, theregister.com (Gemini CLI retirement). OpenCode/Continue/Cline: opencode.ai/docs, docs.continue.dev, cline.bot, docs.cline.bot, github.com/cline/cline. Goose/Amp/Crush/entrants: goose-docs.ai, ampcode.com, github.com/charmbracelet/crush, zed.dev/acp, warp.dev, factory.ai/news (terminal-bench), lucumr.pocoo.org (Pi OSS), openhands.dev, github.com/bradAGI/awesome-cli-coding-agents, Anthropic 2026 Agentic Coding Trends Report.
