# AgilePlus Master Configuration

- **Project:** ForgeCode
- **Current version:** 2.13.21
- **Sprint length:** 7 days, Monday through Sunday
- **Sprint 47:** 2026-08-19 through 2026-08-26
- **Planning cutoff:** 2026-08-19
- **Scorecard baseline:** 2026-08-19 audit, average 6.81 / grade 7.0

## Cadence

| Activity | Cadence | Owner | Output |
| --- | --- | --- | --- |
| Backlog refinement | Weekly | Product owner | `agileplus/backlog.md` |
| Sprint planning | Weekly | Maintainers | `agileplus/sprint-current.md` |
| Daily delivery check | Daily | Sprint owner | Blocker and risk update |
| Mid-sprint review | Wednesday | Maintainers | Scope and gate review |
| Sprint review and retro | Friday | Team | Accepted work and action items |
| Pillar assessment | Weekly | Quality lead | `agileplus/pillars/31-pillar-scorecard.json` |
| Branch cleanup | Nightly | Repository admins | GitHub Actions summary |

A sprint ends only when its acceptance criteria and required quality gates are satisfied. Carryover is re-estimated before the next sprint; completed scope is not silently moved.

## Quality Gates

The authoritative commands are in `agileplus/quality-gates.yml`. Pull requests and release builds must pass:

1. Rust formatting and Clippy with warnings denied.
2. Workspace tests, including the repository's established snapshot workflow where applicable.
3. Dependency and license policy checks.
4. Security scanning and secret detection.
5. Release, packaging, and cross-platform build checks required for the change.

A failed required gate blocks merge. Exceptions require a linked issue, owner, expiry date, documented risk, and maintainer approval.

## Pillar Scorecards

The 31 pillars are grouped into product quality, delivery, community, and governance. The baseline target is 8.0 in every pillar. Scores use this scale:

- **9-10:** Exemplary and evidenced
- **7-8:** Effective; focused improvements only
- **5-6:** Partially effective; remediation planned
- **0-4:** Material gap; active work required

Update the scorecard when evidence changes, but at least weekly. Every change must cite an issue, PR, workflow, release, or document and record the assessment date.

## Working Agreements

- Keep one active sprint goal and a dependency-aware backlog.
- Link work to acceptance criteria and a pillar.
- Keep PRs focused; prefer small, reviewable changes.
- Delete merged or inactive branches only through the guarded nightly workflow.
- Treat accessibility, localization, security, and community health as product quality work.
- Record decisions and carryover rather than relying on chat history.
