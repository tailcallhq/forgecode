---
name: github-pr-comments
description: >
  Resolve inline code review comments on a GitHub PR. Use when asked to
  "resolve review comments", "address PR feedback", "fix PR comments", or
  "work through review comments". Fetches every inline comment with its
  surrounding code context, then applies each change systematically.
---

# Resolve Code Review Comments

## 1. Fetch all comments

Run the bundled script to get every inline comment with its diff hunk:

```bash
bash .forge/skills/resolve-code/scripts/pr-comments.sh [PR_NUMBER]
```

Omit `PR_NUMBER` to use the current branch's PR.

Each block in the output contains:

- `File   :` — file path and line number
- `-- code context --` — the diff hunk showing surrounding lines
- `-- comment --` — the reviewer's message

## 2. Create a todo item per comment

Add one todo for each comment before touching any code. This ensures nothing
is missed even when comments span many files.

## 3. Apply each comment

Work through todos one at a time. There are two comment types:

### Suggestion block

Body starts with ` ```suggestion `. Apply the suggested text verbatim as a
replacement for the highlighted lines in the diff hunk.

### Free-form feedback

Read the comment in the context of the diff hunk, infer the required change,
and implement it. When the intent is ambiguous, make the change that best
matches the project's conventions and state the assumption clearly.

## 4. Verify

After all comments are addressed, run:

```bash
cargo check && cargo nextest run
```

Fix any errors before marking the task complete.
