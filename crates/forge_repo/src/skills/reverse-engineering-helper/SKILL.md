---
name: reverse-engineering-helper
description: >-
  Generic reverse-engineering workflow for understanding an existing system,
  artifact, interface, format, protocol, behavior, or reference implementation
  when documentation is missing, incomplete, or insufficient. Use when asked to
  infer how something works, match existing behavior, interoperate with an
  unknown component, decode inputs/outputs, or reconstruct requirements from
  observed examples.
---

# Reverse Engineering Helper

Understand the unknown system by observing evidence, forming hypotheses, and validating them with small reproducible experiments.

## Workflow

1. Restate the target behavior or compatibility goal.
2. Inventory the available evidence: source, binaries, logs, samples, tests, fixtures, configs, schemas, docs, traffic, user examples, and any domain conventions named by the task.
3. Identify the observable contract: inputs, outputs, side effects, errors, ordering, limits, and invariants.
4. Create the smallest reproducible experiment that exercises one behavior at a time.
5. Compare observations against a baseline, oracle, prior version, reference artifact, generated fixture, second parser/tool, domain convention, or manually derived expectation.
6. Record hypotheses, evidence, and confidence; discard hypotheses that fail concrete checks.
7. Implement or document only the behavior supported by evidence, and call out unresolved unknowns.
8. For recovery, forensics, corrupted/encrypted artifacts, logs, WAL/journal replay, or other evidence-reconstruction tasks, preserve original artifacts or copies before mutating them and derive recovered values from source evidence rather than newly synthesized state.
9. For named scientific, measurement, or domain-specific quantities, separate raw feature detection from semantic assignment; use known units, expected ranges, and plausibility checks before trusting local maxima or heuristics.

## Investigation Checklist

Use only the checks relevant to the task:

- Inputs: accepted types, shapes, encodings, required fields, optional fields, defaults, and invalid cases
- Outputs: format, ordering, precision, determinism, side effects, errors, and compatibility expectations
- Boundaries: empty values, minimum/maximum sizes, unusual characters, malformed data, and version differences
- State: caches, persistence, hidden configuration, environment variables, feature flags, and global state
- Dependencies: external services, libraries, file formats, protocols, schemas, and runtime assumptions
- Algorithms: transformations, normalization, parsing, ranking, scoring, compression, serialization, or validation rules
- Failure behavior: exceptions, fallback paths, retries, partial results, cleanup, and user-visible messages
- Compatibility: backward/forward compatibility, migration behavior, legacy quirks, and exact-match requirements
- Performance: latency, throughput, memory usage, batching, concurrency, and scale-sensitive behavior
- Security and safety: trust boundaries, permissions, secrets, unsafe parsing, and unvalidated external input

## Practical Tactics

- Start with black-box observation before deep implementation analysis.
- Generate paired input/output examples, including simple, boundary, realistic, and at least one fresh input for reusable command-line tools or libraries when feasible.
- Change one variable at a time so each observation maps to a specific cause.
- Break complex behavior into stages and find the first point where expectations diverge.
- Prefer mechanical comparisons: diffs, checksums, logs, snapshots, traces, or structured assertions.
- For corrupted, encrypted, compressed, database, journal/WAL, log, or forensic artifacts, first copy the originals and record sizes, checksums, and headers; perform destructive repair or replay only on copies unless the task explicitly requires replacing the original.
- Avoid self-confirming verification: do not create or insert data and then use that mutated source as the oracle. Compare final outputs against independent evidence from the original artifact, read-only decoded data, verifier-adjacent expectations, domain conventions, a clean copy, or a second extractor/parser.
- Time-box open-ended exploration; if experiments stop reducing uncertainty, pivot to a simpler hypothesis or document the limitation.
- Preserve reproducibility: keep commands, inputs, artifacts, versions, and environment details with the findings.

## Output Format

Return a concise investigation summary or plan:

```markdown
## Goal
- ...

## Evidence
- ...

## Observed contract
- Inputs:
- Outputs:
- Invariants / side effects:

## Hypotheses and checks
- [ ] Hypothesis: ...
  - Check: command, sample, or inspection step
  - Expected evidence:

## Findings
- ...

## Unknowns / risks
- ...

## Next action
- ...
```

Stay generic. Do not assume a specific language, binary format, file type, model architecture, protocol, or implementation strategy unless the task provides one.
