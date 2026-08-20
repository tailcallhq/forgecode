# Prioritized Backlog

Priorities: **P0** blocks release or materially threatens users/security; **P1** is next-sprint work; **P2** scheduled improvement; **P3** opportunistic or deferred. Items remain ordered within each priority until evidence supports a change.

| Rank | Priority | Item | Pillar | Estimate | Dependencies | Acceptance criteria |
| ---: | --- | --- | --- | --- | --- | --- |
| 1 | P0 | Review and run nightly stale-branch cleanup | Branch Mgmt | 3 | API access, dry-run review | Inactive 90+ day and merged unprotected branches are deleted; protected/default branches are retained; run is auditable |
| 2 | P0 | Verify critical security and dependency gates | Security, CI/CD, Deps, Vuln Disc | 5 | CI telemetry | Required scans run on relevant events and produce no unaccepted critical findings |
| 3 | P0 | Define release rollback and hotfix runbook | Releases, Security, Docs | 5 | Release owners | Operators can identify, publish, and verify a rollback from documented commands |
| 4 | P1 | Add release health dashboards and alerts | Monitoring, Logging, Errors | 8 | Metrics backend | Alert thresholds cover errors, latency, crashes, and dependency failures with owners and runbook links |
| 5 | P1 | Improve contributor issue and PR triage labels | Issue Tracking, Reviews, Agile PM | 3 | Maintainer review | Every open item has type, priority, pillar, owner, and lifecycle state |
| 6 | P1 | Establish accessibility release checks | Accessibility, Tests, CI/CD | 8 | Tool selection | A repeatable automated check runs in CI and a documented manual keyboard/screen-reader review remains required |
| 7 | P1 | Create localization ownership and readiness policy | i18n, Accessibility, Docs | 5 | Product owner | Supported locales, fallback behavior, ownership, and release exit criteria are documented |
| 8 | P2 | Define mobile support and compatibility policy | Mobile, API, Docs | 5 | Product requirements | Supported clients, platform lifecycle, and deprecation process are explicit |
| 9 | P2 | Add API compatibility and deprecation checks | API, Tests, Releases | 8 | Public API inventory | Relevant public API changes are checked before release and deprecations include a migration path |
| 10 | P3 | Add community onboarding and recognition flow | Community, CoC, Contributing, Agile PM | 5 | Maintainer availability | New contributors receive issue, development, review, and conduct guidance from a maintained checklist |

## Intake Rules

- New P0 work requires a documented incident, release risk, or security impact.
- P1 work must fit within the next sprint or be split into smaller items.
- Estimates are ideal days; re-estimate after evidence and discovery.
- Link completed work to the applicable pillar and quality gate.
