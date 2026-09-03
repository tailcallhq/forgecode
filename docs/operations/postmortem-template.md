# Postmortem: <short incident title>

> Copy this file to `docs/operations/postmortems/YYYY-MM-DD-<slug>.md` and fill
> it in. Postmortems are **blameless**: focus on systems and gaps, not people.
> ForgeCode is a local tool, so "impact" usually means *users whose installs
> were affected by a release/regression*, not hosted downtime.

## Metadata

| Field | Value |
|-------|-------|
| Incident ID | YYYY-MM-DD-<slug> |
| Status | draft / under-review / final |
| Severity | SEV1 (data loss / credential exposure) / SEV2 (broken for many users) / SEV3 (degraded) / SEV4 (minor) |
| Author(s) | |
| Date detected | |
| Date resolved | |
| Affected versions | e.g. v3.8.30–v3.8.37 |
| Affected surfaces | credentials / tool-exec / MCP / telemetry / shell / daemon / provider |

## Summary

> 2–4 sentences a non-expert can understand: what broke, who was affected, how
> long, and how it was fixed.

## Impact

- **Who/what was affected:** (which users, which OS, which commands/flows)
- **Scope:** (e.g. % of releases, which channels)
- **Data impact:** (any conversation-history loss from `~/.forge/forge.db`? any
  credential exposure? if credentials were exposed, mark SEV1 and follow the
  security path below)
- **SLO impact:** which objective in `docs/operations/slo.md` was burned
  (startup, local latency, daemon availability, provider/MCP effective success)?

## Timeline (local time, most-recent-last)

| Time | Event |
|------|-------|
| | First symptom / earliest known bad commit or release |
| | Detected (how — user report, CI, crash) |
| | Diagnosed |
| | Mitigation shipped |
| | Resolved / verified |

## Detection

- How was this found? (CI smoke timing, user issue, crash report, audit)
- Should it have been caught earlier? Which signal was missing?

## Root cause

> The technical "why". Cite the code path / commit. Distinguish the *trigger*
> from the *underlying cause*. Example anchors: resilience layer
> (`crates/forge_infra/src/resilience.rs`), credential store
> (`crates/forge_repo/src/provider/provider_repo.rs`), daemon
> (`crates/forge_dbd/src/server.rs`), telemetry
> (`crates/forge_tracker/`).

## Resolution & recovery

- What fixed it (commit/PR link).
- Recovery steps users needed to take (link the relevant
  `docs/operations/runbook.md` section).
- For credential incidents: rotation/revocation performed (see threat-model
  S1 / gap G1.4).

## What went well

-

## What went poorly

-

## Where we got lucky

-

## Action items

| # | Action | Type (prevent / detect / mitigate / process) | Owner | Tracking link | Done |
|---|--------|----------------------------------------------|-------|---------------|------|
| 1 | | | | | [ ] |
| 2 | | | | | [ ] |

> Each SEV1/SEV2 must produce at least one *prevent* and one *detect* action.
> Link new threat-model gaps back into `docs/security/threat-model.md` if the
> incident exposed an unmodeled surface.

## Lessons / threat-model & SLO updates

- New or revised threat-model entries:
- New or revised SLO targets / SLIs:
- Runbook entries added/updated:
