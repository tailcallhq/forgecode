# SLO Burn Rate Alerting — Forgecode

> **Status:** Active
> **Owner:** Forgecode SRE / maintainers
> **Last reviewed:** 2026-08-20

## Overview

Burn rate measures how fast you are consuming your error budget. A burn rate of
1x means you are consuming budget exactly in line with the SLO target (healthy).
A burn rate of 14.4x means you will exhaust your entire monthly error budget in
2 days.

This document defines the multi-window burn-rate alerting strategy used by the
Forgecode CI/CD pipeline to detect SLO violations early.

---

## SLO Definition

| Parameter | Value |
|-----------|-------|
| **SLO Target** | 99.9% CI success rate |
| **Monthly Error Budget** | 43.2 minutes (~0.001 × 30 days × 24 h × 60 min) |
| **Budget Expressed As** | 43.2 minutes of CI failure time per 30-day window |
| **Measurement** | CI workflow run success/failure on `main` branch |

### Why 99.9%?

Forgecode is a local developer tool. CI reliability directly impacts release
cadence and contributor confidence. A 99.9% SLO means at most ~43 minutes of
CI failure per month before the budget is exhausted and a release is blocked.

---

## Burn Rate Windows

The burn rate is calculated over four rolling windows to balance **speed of
detection** against **noise**:

| Window | Purpose | Lookback |
|--------|---------|----------|
| **1 hour** | Fast detection of acute failures (broken `main`, bad merge) | Last 1 hour |
| **6 hours** | Detect sustained issues that 1-hour window might miss | Last 6 hours |
| **24 hours** | Daily trend — catches slow-burn regressions | Last 24 hours |
| **7 days** | Weekly trend — filters out transient flakiness | Last 7 days |

### Calculation

```
burn_rate = (failure_time_in_window / window_duration) / (1 - SLO_target)
```

For a 99.9% SLO:
- A 1x burn rate means `0.1%` failures in the window (healthy)
- A 14.4x burn rate means `1.44%` failures in the window (budget exhausts in 2 days)

---

## Alert Thresholds

| Severity | Burn Rate | Budget Exhaustion | Meaning |
|----------|-----------|-------------------|---------|
| **PAGE** | ≥ 14.4x | 2 days | Critical — budget will be exhausted within 48 hours |
| **WARN** | ≥ 3.0x | 10 days | Warning — budget consumption is unsustainable |
| **INFO** | ≥ 1.0x | 30 days | Informational — at the boundary of the SLO target |

### Multi-Window Requirements (to reduce false positives)

| Alert | Primary Window | Secondary Window |
|-------|----------------|------------------|
| PAGE (14.4x) | 1-hour burn rate ≥ 14.4x | 6-hour burn rate ≥ 14.4x |
| WARN (3.0x) | 6-hour burn rate ≥ 3.0x | 1-day burn rate ≥ 3.0x |
| INFO (1.0x) | 1-day burn rate ≥ 1.0x | 7-day burn rate ≥ 1.0x |

The secondary window prevents flapping from transient failures.

---

## Error Budget Calculation

| Period | Total Minutes | 0.1% Budget | Budget Consumed |
|--------|---------------|-------------|-----------------|
| 1 day | 1,440 min | 1.44 min | Measured by workflow |
| 7 days | 10,080 min | 10.08 min | Measured by workflow |
| 30 days | 43,200 min | 43.2 min | Measured by workflow |

---

## Monitoring

The daily CI workflow `.github/workflows/slo-monitor.yml`:

1. Queries the last 7 days of CI runs via `gh api`
2. Calculates burn rate for each window
3. Computes error budget consumed and remaining
4. Projects the budget exhaustion date
5. Creates a GitHub issue if the burn rate exceeds the WARN threshold

### Summary Table Format

The workflow posts a summary table with:

| Column | Description |
|--------|-------------|
| SLO Target | The target reliability percentage |
| Error Budget Consumed | Minutes/fraction of budget used |
| Remaining Budget | Minutes/fraction of budget left |
| Current Burn Rate | Current burn rate multiplier |
| Projected Exhaustion | Date when budget will be exhausted if burn continues |

---

## Response Playbook

| Burn Rate | Action |
|-----------|--------|
| **≥ 14.4x** | Page on-call. Investigate immediately. Bisect to offending commit. Block all merges until resolved. |
| **≥ 3.0x** | Open high-priority issue. Investigate within 4 hours. Consider rolling back recent changes. |
| **≥ 1.0x** | Review in next standup. Check for flaky tests or known intermittent failures. |

---

## Related Documents

- [SLA-SLO.md](./SLA-SLO.md) — Full SLO targets and measurement methodology
- [incident-response.md](./incident-response.md) — Incident response procedures
