---
name: resolve-fixme
description: Find all FIXME comments across the codebase and fully implement the work they describe. Use when the user asks to fix, resolve, or address FIXME comments, or when running the "fixme" command. Runs a discovery script to find every FIXME, expands multiline comment blocks, groups related FIXMEs across files into a single implementation task, completes the full underlying code changes, removes the FIXME comments only after the work is done, and verifies that no FIXMEs remain.
---

# Resolve FIXME Comments

## Workflow

### 1. Run the discovery script

Execute the script from the repository root to collect all FIXMEs with context:

```
bash .forge/skills/resolve-fixme/scripts/find-fixme.sh [PATH]
```

- `PATH` is optional; omit it to search the entire working directory.
- The script prints each FIXME with **2 lines of context before** and **5 lines after**, along with the exact file path and line number.
- Skips `.git/`, `target/`, `node_modules/`, and `vendor/`.
- Requires either `rg` (ripgrep) or `grep` + `python3`.

### 2. Expand each FIXME into its full instruction

Do not rely on the discovery output alone.

For every hit:

1. Open the file and read around the reported line.
2. Expand the FIXME to include the **entire comment block**.
3. Treat all consecutive related comment lines as part of the same instruction.

Important:

- A FIXME may be **multiline**. The line containing `FIXME` is often only the beginning.
- The real instruction may continue on following comment lines and may contain the actual implementation details.
- Do not interpret or edit a FIXME until you have read the full block.

For each expanded FIXME, capture:

- file path
- start line and end line of the full comment block
- a short summary of what that FIXME is asking for

### 3. Consolidate related FIXMEs across files

Before editing code, review **all** expanded FIXMEs together.

Many FIXMEs describe different facets of the same underlying task across multiple files. For example:

- one file may describe a domain type that needs to be introduced
- another may describe a parameter that should disappear once that type exists
- another may describe a service, repo, or UI update needed to complete the same refactor

Group such FIXMEs into a single implementation task.

When grouping, look for:

- shared vocabulary
- references to the same type, service, repo, parameter, or feature
- comments that clearly describe prerequisite and follow-up changes in different files
- comments that only make sense when read together

For each group, produce one consolidated understanding of the task:

- all files and line ranges involved
- the complete implementation required across the group
- the order in which the changes should be made

Do not resolve grouped FIXMEs one file at a time in isolation. Resolve the whole task consistently.

### 4. Implement every FIXME completely

Every FIXME must be resolved. There is no skip path.

Work through each grouped task until the underlying implementation is complete:

1. Read any additional files needed to understand the design.
2. Create or modify the required code, types, services, repos, tests, configs, or templates.
3. Propagate the change through every affected file in the group.
4. Remove each FIXME comment **only after** the work it describes has actually been implemented.

> **Critical rule:** Never delete or rewrite a FIXME comment unless the underlying implementation is finished. The comment is a record of required work. Removing it before completing that work is a failure.

If the FIXME implies a larger refactor, do the refactor. If it requires creating new supporting code, create it. Do not stop at the first local change if the comment clearly implies additional follow-through elsewhere.

### 5. Verify

After resolving all FIXMEs:

1. Run the project's standard verification step:

```sh
cargo insta test --accept
```

2. Re-run the discovery script:

```sh
bash .forge/skills/resolve-fixme/scripts/find-fixme.sh [PATH]
```

3. Confirm that no FIXME comments remain in the targeted scope.

## Notes

- Prefer targeted fixes, but do not under-scope the work when multiple FIXMEs describe one larger task.
- Read broadly before editing when the intent is ambiguous.
- Consistency matters more than locality: grouped FIXMEs should lead to one coherent implementation.
- The job is not to clean up comments. The job is to complete the implementation those comments are pointing at.
