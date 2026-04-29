---
name: execute-plan
description: Execute structured task plans with status tracking. Use when the user provides a plan file path in the format `plans/{current-date}-{task-name}-{version}.md` or explicitly asks you to execute a plan file.
arguments: [plan_path]
---

# Execute Plan

Execute structured task plans with automatic status tracking and progress updates.

## Plan Path Input

Execute the plan at: `$plan_path`

## Commitment to Completion

When a plan is provided, **all tasks in the plan must be completed**. Before starting execution, recite:

> "I will execute this plan to completion. All the 20 tasks will be addressed and marked as DONE."

## Execution Steps

**STEP 1**: Recite the commitment to complete all tasks in the plan.

**STEP 2**: Read the plan file at `$plan_path` and identify all pending tasks based on `task_status`.

**STEP 3**: Announce the next pending task and update its status to `IN_PROGRESS` in the plan file.

**STEP 4**: Execute all actions required to complete the task and mark the task status to `DONE` in the plan file.

**STEP 5**: Repeat from Step 3 until all tasks are marked as `DONE`.

**STEP 6**: Re-read the plan file to verify all tasks are completed before announcing completion.

## Task Status Format

Use these status indicators in the plan file:

```
[ ]: PENDING
[~]: IN_PROGRESS
[x]: DONE
[!]: FAILED
```

## Example Usage

1. User provides: "Execute plan at plans/2025-11-23-refactor-auth-v1.md"
2. Or skill invoked with arguments: `plans/2025-11-23-refactor-auth-v1.md`
3. Recite commitment: "I will execute this plan to completion..."
4. Resolve plan path from request/arguments
5. Read the plan file
6. Find first `[ ]` (PENDING) task
7. Update to `[~]` (IN_PROGRESS)
8. Execute the task
9. Update to `[x]` (DONE)
10. Move to next PENDING task
11. Repeat until all tasks appear DONE
12. Re-read plan file to verify completion
13. Announce completion
