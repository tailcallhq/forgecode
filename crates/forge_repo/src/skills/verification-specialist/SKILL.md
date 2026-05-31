---
name: verification-specialist
description: >-
  Verify your solution works correctly before completing the task.
  Call AFTER implementing and BEFORE claiming completion.
---

# Verification Specialist

## Checklist

1. Use the **verification-matrix** (from system reminders) as your checklist
2. Verify every required output file exists at the exact path
3. Run a real smoke test that exercises the deliverable end-to-end through the required external interface
4. Prefer read-only checks for existing deliverables; if a smoke test must mutate state, mutate it only through the task's required interface
5. Before cleanup, classify generated paths, processes, refs, records, services, listeners, and build/test outputs as required final state, verification byproduct, or unrelated scratch; if uncertain, preserve it
6. Clean up only verification byproducts and scratch; never reset, delete, revert, stop, or overwrite final artifacts or persistent state required by the task or produced by the required workflow
7. For single-file, clean-directory, or exact-manifest tasks, keep verification byproducts outside the deliverable when possible and re-check the final manifest after cleanup
8. For deployment, service, VM/emulator, container, daemon, or network-interface tasks, verify the full protocol contract with a fresh client: handshake, authentication if any, request/command execution, and response content. A listening port alone is not sufficient.
9. For functions, libraries, or CLIs, run at least one caller-style check using the public signature/command and representative arguments; when the interface is under-specified, test reasonable positional/named argument conventions or generated inputs when feasible.
10. For exact text, transcription, OCR, or visual-decoding deliverables, verify content with an independent mechanical source when feasible; install lightweight OCR/extraction tooling if available, and check confusable characters, case, punctuation, and whitespace
11. For recovery/forensics tasks, preserve originals/copies before mutation and verify recovered output against original evidence, a read-only decoded copy, verifier-adjacent expectations, or a second parser/extractor rather than synthesized data
12. For numeric, ML, scientific, ranking, or heuristic outputs, include a semantic plausibility check from canonical docs/domain conventions or a small independent/reference calculation; do not verify only schema or your chosen hypothesis
13. For performance-sensitive tasks, benchmark the full declared input domain and boundary/intermediate cases rather than only public sample sizes; repeat measurements, prefer fresh-process checks when verifier isolation is likely, and require a stable safety margin over the reference or threshold
14. If multiple plausible interpretations produce different outputs, resolve the ambiguity using prompt wording, discoverable docs, canonical APIs, domain conventions, or broad compatibility before finalizing
15. Leave verification output in the transcript
