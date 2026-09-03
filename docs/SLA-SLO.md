# Service Level Agreement (SLA) and Service Level Objectives (SLO) — Forgecode

> **Status:** Living document.
> **Owner:** Forgecode maintainers.
> **Last reviewed:** 2026-08-19.

## Overview

Forgecode (HeliosLite) is a **local CLI/TUI developer tool** — not a hosted service.
There is no fleet, no load balancer, and no 24/7 availability target. Classic
uptime SLAs do not apply. Instead, the SLOs in this document capture the
qualities a developer feels: **startup speed, command responsiveness, memory
footprint, and binary portability.**

"Error budgets" are adapted for a local tool: rather than minutes of downtime,
the budget is the **fraction of user-initiated operations allowed to miss the
target before the regression is treated as a release blocker.**

For daemon-specific and provider-resilience SLOs, see `docs/operations/slo.md`.

---

## SLA Targets

| Metric | Target | Error Budget | Notes |
|--------|--------|--------------|-------|
| **Cold startup** | p95 < 400 ms | 5 % of cold starts may exceed p95 | Excludes first-ever run (asset extraction, plugin install) |
| **Warm command** | p95 < 100 ms | 5 % | Config read, history list, shell completions — no network call |
| **First token overhead** | p95 < 150 ms | 5 % | Forge-attributable overhead only; provider network time excluded |
| **Memory footprint** | < 200 MB RSS | N/A — hard ceiling | Measured after warm-up; excludes provider response streaming buffers |
| **Binary size** | < 10 MB (release, stripped) | N/A — hard ceiling | Linux x86_64; platform-specific builds may vary slightly |
| **Availability** | N/A | N/A | Local tool — availability is the developer's machine health |

---

## Measurement Methodology

### Cold startup

Wall-clock time from `exec("forge")` to the first interactive prompt or usable
output on a "warm filesystem" (binary already cached by the OS, no prior `forge`
process). Measured by CI smoke tests and the `benchmarks/perf_harness` crate.

### Warm command

Wall-clock time for a local-only command that does not issue a provider call
(e.g. `forge config show`, `forge history list`). Measured via TUI
instrumentation and the `benchmarks/dual_harness` crate.

### First token overhead

Time from prompt submission to the first streamed token from the provider,
**minus** the provider-side latency. The forge-attributable portion (prompt
assembly, HTTP framing, SSE deserialization) is isolated by replaying cached
provider responses.

### Memory footprint

Peak RSS of the forge process after a 60-second warm-up period, measured with
`sysinfo` (Rust) or `/proc/self/status` on Linux. CI collects this via the
`perf_harness` benchmark suite and records it in `agileplus/perf-trend-history.json`.

### Regression detection

The CI workflow (`.github/workflows/perf-trend.yml`) compares each run against
the baseline stored in `agileplus/perf-trend-history.json`. A **>20 %
regression** on any metric fails the check and blocks the workflow.

---

## Escalation

Because Forgecode is a local tool, "escalation" means **triage priority** when a
regression is detected:

1. **P0 — Cold startup > 800 ms (2× target):** Block merge. Investigate
   immediately; bisect to the offending commit.
2. **P1 — Any SLO metric misses target by > 50 %:** Flag in PR review. Must be
   resolved before the next release.
3. **P2 — Trend drift of 10–20 % without a single breaking commit:** Schedule
   investigation in the next sprint. Likely cumulative bloat.

### Contacts

| Priority | Escalation path |
|----------|-----------------|
| P0 | PR author + on-call reviewer (GitHub `@KooshaPari`) |
| P1 | Forgecode maintainers team |
| P2 | `#forgecode-perf` discussion channel |

---

## Review Schedule

- **Per PR:** CI automatically checks for >20 % regression against baseline.
- **Monthly:** Review `perf-trend-history.json` for drift; update baselines if
  structural changes intentionally shift the curve.
- **Quarterly:** Audit this document against current tool behavior; update
  targets if the user-facing surface changes materially.
