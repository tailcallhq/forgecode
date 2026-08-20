# Journey Manifests

Machine-readable, outside-in user journeys. Each manifest walks one real
user-facing flow end-to-end using only documented commands, with an expected
result and a verification command per step. They are the executable form of
[the journey-traceability standard](../../operations/journey-traceability.md)
and each journey maps to one or more FR-IDs from
[`FUNCTIONAL_REQUIREMENTS.md`](../../../FUNCTIONAL_REQUIREMENTS.md).

## Format

```yaml
journey: <kebab-case-id>
title: <human title>
fr: [FR-NNN, ...]           # linked functional requirements
steps:
  - step: <name>
    command: <exact shell command>
    expect: <observable expected result>
    verify: <grep/exit-code assertion to run after the command>
```

## Manifests

| Journey | Flow | FRs |
|---|---|---|
| [onboarding-journey.md](onboarding-journey.md) | install → first session → config → first tool call | FR-001, FR-004, FR-009, FR-011 |

## Status

- [x] Identify key user-facing flows (onboarding seeded; config, MCP, update journeys next)
- [x] Author manifests in `docs/journeys/manifests/`
- [ ] Record VHS tapes for each flow
- [ ] Run `phenotype-journey verify` in CI
