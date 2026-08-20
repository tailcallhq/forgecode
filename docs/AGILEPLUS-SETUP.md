# AgilePlus setup

AgilePlus gives ForgeCode a lightweight operating system for delivery quality. The baseline reflects the 2026-08-19 audit: 31 pillars, average 6.81, grade 7.0, 386 releases, and 275 remote branches.

## What was added

- `agileplus/AGILEPLUS.md` defines the weekly cadence, quality gates, pillar model, and working agreements.
- `sprint-current.md` tracks sprint 47 (2026-08-19 through 2026-08-26), its goal, acceptance criteria, risks, and review template.
- `backlog.md` orders the first ten P0-P3 improvements.
- `pillars/31-pillar-scorecard.json` stores the auditable baseline and target of 8.0 per pillar.
- `quality-gates.yml` records format, Clippy, tests, coverage, dependency, security, and release requirements.
- `velocity.md` provides the rolling five-sprint measurement table and capacity rules.
- `CODEOWNERS-pillars` maps repository surfaces to the current maintainer until pillar teams are appointed.

The root `CODE_OF_CONDUCT.md` adopts Contributor Covenant v2.1. `CHANGELOG.md` is initialized from the ten latest GitHub releases and the latest 100 commits.

## Automation

### Weekly scorecard

`.github/workflows/agileplus-pillar-scorecard.yml` runs every Monday at 09:17 UTC. It validates the scorecard JSON, renders all 31 pillars, and comments on the `AgilePlus Pillar Scorecard` issue. If the issue does not exist, the workflow creates it. Set the repository variable `AGILEPLUS_SCORE_CARD_ISSUE` to an existing issue number to pin updates to that issue.

### Nightly branch cleanup

`.github/workflows/branch-cleanup.yml` runs at 02:27 UTC. It never deletes the default branch, skips branches with open pull requests, verifies branch protection before deletion, and only considers branches whose latest commit is at least 90 days old or whose pull request has been merged. Protected branches are retained.

The workflow is dry-run by default through the manual dispatch input. To enable deletion, pass `dry_run: false`; the repository variable `AGILEPLUS_BRANCH_CLEANUP_DRY_RUN=false` can provide the same setting for scheduled runs. Review the generated run summary before enabling automatic deletion.

## Maintainer loop

1. Assign a named owner to each sprint and backlog item.
2. Replace velocity `TBD` values from completed sprint records.
3. Run the weekly scorecard and link evidence for score changes.
4. Review branch-cleanup output while the remote set is reduced from 275 branches.
5. Promote the 31 pillar groups into the existing `CODEOWNERS` as named teams are created.
