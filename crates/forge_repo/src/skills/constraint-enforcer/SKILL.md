---
name: constraint-enforcer
description: >-
  Quick pre-flight for tasks with constraints. Call if the prompt has:
  size limits, exact paths, output formats, anti-shortcut rules, or
  parameter coverage requirements.
---

# Constraint Enforcer

30 seconds max. Extract constraints into a numbered list, then start coding.

```markdown
CONSTRAINTS:
1. Required deliverables: exact paths, output formats, field names, and final-state requirements.
2. Runtime interface: command/function/service call an outside checker would use, including arguments and protocol/auth details.
3. Semantic success: what must be true about the content/result, not only that files exist or commands run.
4. Ambiguities: plausible interpretations and the default resolution (canonical docs/API/domain convention or broad compatibility).
5. Mutation risks: original artifacts or persistent state that must be copied/preserved before inspection or repair.
6. Verification path: one direct verifier-equivalent command/check to run before completion.
```

Then start coding. Keep this pre-flight concise; do not turn it into a long plan.
