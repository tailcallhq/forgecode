# Forge Cursor Position Error Investigation & Fix
# Forge Cursor Position Error Investigation & Fix

## Problem
Multiple forge sessions in `repos` are crashing with **"cursor position could not be read in a normal duration"** error.

## Initial Findings

### 1. Cursor Tracking in Codebase
- **`executor.rs:208`**: There's a comment noting flush is necessary to avoid "cursor could not be found" errors
- **Terminal Context**: Reads from zsh plugin environment variables (`_FORGE_TERM_COMMANDS`, etc.)
- **UI Cursors**: These are fzf/select widget cursors, NOT terminal cursor position

### 2. Error Location Unknown
- The error message **"cursor position could not be read in a normal duration"** is NOT found in the Rust source
- Likely comes from:
  - Upstream ForgeCode binary (pre-compiled)
  - Terminal/TTY layer
  - zsh plugin hooks

### 3. Session State in Database
- **4161 total conversations** in `~/forge/.forge.db`
- Sessions crash but don't properly clean up
- Need to audit for incomplete/orphaned sessions

## Session Audit Results (Last 24 Hours)

### Summary
- **Total conversations**: 15
- **Completed**: 10 (67%)
- **Likely Incomplete**: 2 (13%)
- **Unknown/Needs Review**: 3 (20%)

### Sessions Needing Resumption

| ID | Title | Issue |
|----|-------|-------|
| `ddeddf14` | Audit and stabilize `thegent` | **CRASHED** - Last message cut off mid-sentence. Likely cursor position error. |
| `efa9e0a4` | Audit thegent (task plan) | Task plan created but work not started |
| `9193766b` | Extract GitHub repos/papers | Download still in progress (23%) |

### Sessions Completed (but no TASK COMPLETED marker)
- `f1dcf57b` - PolicyStack tests (All 513 tests pass)
- `e5193bbc` - Identify incomplete sessions (investigation complete)
- `1e984679` - Idle forge sessions (table complete)
- `30b57666` - SOTA helios-cli (document exists)
- Plus 6 others with proper completion markers

---

## Investigation Tasks

- [x] 1. **Audit all forge conversations**: Found 3 sessions needing resumption
- [ ] 2. **Find the error source**: Search upstream ForgeCode binary or check if it's from terminal TTY
- [ ] 3. **Check zsh plugin hooks**: Review `preexec`/`precmd` hooks for cursor tracking
- [ ] 4. **Examine TTY/terminal code**: Look for `TIOCGWINSZ` or cursor position reads
- [ ] 5. **Review task cancellation timing**: Check if async task cancellation affects cursor state
- [ ] 6. **Check parallel tool execution**: Look for race conditions in cursor tracking

## Potential Fixes

1. **Deterministic flush ordering** - Ensure explicit flush after all output
2. **Cursor state machine** - Track cursor state transitions with proper guards
3. **Graceful degradation** - Timeout handling when cursor can't be read
4. **Race condition fixes** - Proper synchronization for parallel operations
5. **Error recovery** - Add retry logic for cursor position reads

## Verification

- [ ] Add integration tests for cursor tracking under load
- [ ] Test with long-running commands
- [ ] Test with multiple parallel tool calls
- [ ] Verify fix with batch session resumption

## Status
**Investigating** - Error source not yet located in codebase
