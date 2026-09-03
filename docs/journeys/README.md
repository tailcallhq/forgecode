# Journey Manifests

Outside-in acceptance for forgecode's most important user flows. A **journey
manifest** is a stepwise, executable description of one real user-facing flow
from the shell — install, first run, configuration, first tool call — with an
expected result and a verification step for every command. Manifests live in
[`manifests/`](manifests/README.md).

Why: the scorecard (C03, L30.6) flagged that the journey-traceability standard
was referenced but no executable journeys existed. Manifests make the
acceptance contract in [`FUNCTIONAL_REQUIREMENTS.md`](../../FUNCTIONAL_REQUIREMENTS.md)
testable outside-in and are the input to `phenotype-journey verify`
(see [`docs/operations/journey-traceability.md`](../operations/journey-traceability.md)).

## Format

Every manifest is a YAML file with this shape:

```yaml
journey: <kebab-case-id>
title: <human title>
fr: [FR-NNN, ...]          # IDs from FUNCTIONAL_REQUIREMENTS.md
steps:
  - step: <name>
    command: <exact shell command, copy-pasteable>
    expect: <observable expected result>
    verify: <assertion — grep, exit code, or file existence check>
```

## Rules

1. **Only documented commands.** Every `command` must appear in the README,
   `docs/`, or `forge --help` output — manifests must not invent flags.
2. **One observation per step.** `expect` must be verifiable without
   guesswork.
3. **Link FRs.** Each manifest lists the FR-IDs it exercises.
4. **Idempotent when possible.** Setup steps (config, setup) must be safe to
   re-run.
5. **No secrets.** Commands never embed credentials; they reference the
   `~/.forge` credential store or env vars.

## Authoring a new journey

1. Pick a flow from the traceability standard
   (`docs/operations/journey-traceability.md`).
2. Walk the flow yourself in a scratch shell and record exact commands,
   expected results, and verification greps.
3. Add the manifest under `manifests/` and index it in
   [`manifests/README.md`](manifests/README.md).
4. Reference the FR-IDs it covers so the catalog stays the source of truth.

## Status

- [x] Manifest format + first journey ([onboarding](manifests/onboarding-journey.md))
- [ ] Journeys for config (`FR-004`), MCP (`FR-008`), update (`FR-010`)
- [ ] VHS tapes per flow
- [ ] `phenotype-journey verify` wired into CI
