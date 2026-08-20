# ForgeCode Service Level Objectives (SLOs)

> Status: Living document. Owner: ForgeCode maintainers.
> Last reviewed: 2026-06-28 (Phase P4 overhaul).

ForgeCode is a **local CLI/TUI plus an on-demand local daemon** (`forge_dbd`),
not a hosted service. Classic uptime SLAs do not apply: there is no fleet, no
load balancer, no 24/7 availability target. Instead we define SLOs for the
qualities a developer actually feels — **startup latency, command
responsiveness, daemon availability on the local box, and resilience to flaky
upstream providers.**

These SLOs are aspirational targets used to triage regressions and prioritize
work. The "error budget" framing is adapted for a CLI: rather than minutes of
downtime, the budget is the **fraction of user-initiated operations allowed to
miss the target before it is treated as a regression.**

## SLI definitions

| SLI | Definition | How it's measured |
|-----|------------|-------------------|
| **Startup time** | Wall-clock from process exec to interactive prompt / first usable output | `time forge` cold; CI smoke timing |
| **Command latency (local)** | Wall-clock for a local-only command (no provider call): config read, history list, completion | TUI instrumentation / manual timing |
| **First token latency** | Time from submitting a prompt to first streamed token from the provider | depends on provider; forge overhead measured separately |
| **Daemon availability** | Fraction of `Ping` health probes to `forge_dbd` that succeed when the daemon should be up | `Request::Ping` → `Response::Health` round-trip |
| **Provider call success (effective)** | Fraction of provider calls that eventually succeed *after* retry/circuit-breaker handling | retry layer outcome |
| **MCP call success (effective)** | Fraction of MCP tool calls that succeed without being shed by the bulkhead/breaker | circuit breaker `"mcp_client"` outcome |

## SLO targets and error budgets

| Objective | Target | Error budget | Notes |
|-----------|--------|--------------|-------|
| Cold startup | p95 < 400 ms; p99 < 800 ms | 5% of starts may exceed p95 | Excludes first-ever run (asset extraction, plugin install) |
| Warm local command | p95 < 100 ms | 5% | Config/history/completion; no network |
| Daemon availability (local) | 99% of probes succeed while daemon enabled | 1% | Daemon is optional; absence is *not* a budget burn (see below) |
| Provider call effective success | 99% over a rolling session | 1% | After retry of `[429, 500, 502, 503, 504, 408, 522, 524, 520, 529]` |
| MCP call effective success | 95% | 5% | Bulkhead shedding under saturation counts as a miss |
| First token latency overhead (forge-attributable) | p95 < 150 ms | 5% | Provider network time is excluded |

### Why "daemon down" is not always a budget burn

`forge_dbd` is a **best-effort persistence accelerator** (conversation storage
via SQLite at `~/.forge/forge.db`, Unix socket at `~/.forge/.forge.db.sock` —
`crates/forge_dbd/src/main.rs:15-23`). Client-side conversation storage is
split by default: writes land in `~/.forge/.forge.writes.db` and reads union
the legacy `~/.forge/.forge.db` via the `conversations_all` TEMP VIEW. forge is
designed to degrade gracefully when the daemon is absent. The availability SLO applies only while the daemon is
*expected* to be running; a deliberately-disabled daemon does not consume the
budget. A daemon that is *supposed* to be up but unreachable **does** (see the
runbook for the "daemon down" procedure).

## Resilience behavior backing these SLOs

The provider/MCP success SLOs are achievable because of the Phase-P2 resilience
layer:

- **Unified retry** on a single `RetryConfig`
  (`crates/forge_config/src/retry.rs:6-26`) with exponential backoff and a
  default retryable status set
  `[429, 500, 502, 503, 504, 408, 522, 524, 520, 529]`. Provider-specific
  overload errors (Anthropic `overloaded_error`, OpenAI `server_is_overloaded`)
  are also retried (`crates/forge_repo/src/provider/retry.rs:9-37`).
- **Circuit breaker** (`crates/forge_infra/src/resilience.rs:47-132`):
  Closed → Open after 5 consecutive failures, Open → HalfOpen after a 30s reset
  timeout, HalfOpen → Closed on a successful probe. This caps wasted latency
  when a provider/MCP server is hard-down (fast-fail rather than per-call
  timeout).
- **Bulkhead** (`resilience.rs:195-242`): a non-blocking semaphore caps
  concurrent MCP calls (default 16) and returns `BulkheadFullError` immediately
  on saturation, protecting the rest of the agent from one slow server.

## Measuring & reviewing

- **CI smoke timing** should assert cold-startup stays under the p99 budget on
  the standard Linux runner; a regression fails the budget.
- **Local instrumentation**: the TUI/daemon emit timing for local commands and
  daemon round-trips; spot-check against targets when investigating "feels
  slow" reports.
- **Provider/MCP effective success** is observed via the retry/breaker outcomes;
  a breaker that trips frequently in normal use is an SLO regression, not just a
  provider problem — see the runbook circuit-breaker section.

Review this file whenever startup cost materially changes (new asset extraction,
new always-on subsystem) or when a new latency-sensitive surface is added.
