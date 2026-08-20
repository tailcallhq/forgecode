# Sprint 47

**Dates:** 2026-08-19 → 2026-08-26
**Status:** Active
**Capacity:** To be confirmed at planning
**Review:** 2026-08-26

## Goal

Raise release confidence and operational maturity while reducing repository hygiene risk from stale branches and weak observability.

## Committed Work

| Priority | Work item | Acceptance criteria | Pillar | Owner |
| --- | --- | --- | --- | --- |
| P0 | Establish branch cleanup | Nightly cleanup removes only stale or merged, unprotected branches; summary reports decisions | Branch Mgmt | TBD |
| P0 | Publish weekly pillar scorecard | Workflow posts an auditable scorecard to the AgilePlus GitHub issue | Agile PM, CI/CD | TBD |
| P1 | Define release health signals | Alerts and dashboards cover crash, latency, error-rate, and dependency thresholds | Monitoring | TBD |
| P1 | Raise localization baseline | Localization status, owners, and a measurable release-readiness target are documented | i18n, Accessibility | TBD |
| P1 | Improve issue lifecycle hygiene | Issue templates and labels distinguish bug, feature, pillar, and sprint work | Issue Tracking, Community | TBD |

## Quality Gates

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo insta test --accept`
- `cargo deny check`
- `cargo llvm-cov --all-features --workspace --fail-under-lines 85`
- Security and secret scans required by the CI workflow
- Release builds on all platforms in the release matrix

## Risks and Dependencies

| Risk/dependency | Impact | Mitigation | Owner |
| --- | --- | --- | --- |
| No named pillar owners yet | Reviews and score updates may be delayed | Assign owners during planning | TBD |
| 275 remote branches | Nightly API volume and accidental deletion risk | Default dry run until policy is reviewed | TBD |
| Incomplete monitoring evidence | Grade 6 cannot be independently verified | Link dashboards or define acceptance checks | TBD |

## Daily Check

Record only changes to scope, blockers, gate failures, and carryover risk. Update the backlog when priorities or estimates change.

## Sprint Review Notes

_To be completed at review._

## Carryover

_To be completed at review._
