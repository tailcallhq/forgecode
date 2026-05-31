---
name: edge-case-tester
description: >-
  Generic edge-case analysis for any task, feature, bug fix, or implementation
  request. Use when asked to identify edge cases, boundary cases, adversarial
  scenarios, hidden assumptions, or before implementation/testing to turn the
  task description into a concise list of cases that should be considered.
---

# Edge Case Tester

Read the task carefully and list the edge cases most likely to break the expected behavior.

## Workflow

1. Restate the core behavior in one sentence.
2. Extract the inputs, outputs, constraints, assumptions, and success criteria from the task.
3. List concrete edge cases to consider, prioritizing cases that are likely to reveal bugs.
4. Include specific example inputs or scenarios whenever possible.
5. Call out any ambiguity that prevents choosing the correct edge cases.

## Edge Case Categories

Consider only categories relevant to the task:

- Empty, missing, null, undefined, or default values
- Minimum, maximum, and just-outside boundary values
- Invalid, malformed, unexpected, or unsupported inputs
- Duplicate, unordered, conflicting, or partially overlapping data
- Very small, very large, or performance-sensitive inputs
- Special characters, whitespace, casing, encoding, locale, and timezone issues
- Permission, authentication, configuration, and environment differences
- Network, filesystem, database, concurrency, retry, and partial-failure scenarios
- Public interface variations: positional vs named arguments, default values, generated inputs, and caller-style CLI/function usage
- Ambiguous schemas or fields where multiple plausible interpretations produce different outputs
- Heuristic/data-processing generalization: additional samples, false positives, off-by-one boundaries, and domain-plausibility checks
- Backward compatibility, migration, and existing-data behavior
- Error handling, recovery, cleanup, and user-facing failure messages

## Output Format

Return a concise checklist:

```markdown
## Core behavior
- ...

## Edge cases
- [ ] Case name: concrete scenario/input and why it matters
- [ ] Case name: concrete scenario/input and why it matters

## Ambiguities
- ...
```

Do not overfit to a specific domain. Derive the edge cases from the task description and include domain-specific cases only when the task implies them.
