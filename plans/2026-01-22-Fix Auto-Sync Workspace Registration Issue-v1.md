# Fix Auto-Sync Workspace Registration Issue

**Issue:** [#2288](https://github.com/tailcallhq/forgecode/issues/2288)

**Problem:** The zsh plugin automatically syncs workspaces in the background on every directory change, causing unintended parent directories to be registered as workspaces. When users run `forge workspace list`, ancestor directories appear as "Current" instead of the actual working directory.

**Root Cause:** `_forge_start_background_sync()` in `shell-plugin/lib/helpers.zsh:114-131` unconditionally runs `forge workspace sync` without checking if the workspace or its ancestors are already registered.

---

## Objective

Prevent auto-sync from creating unintended workspace registrations while maintaining the convenience of automatic syncing for already-indexed workspaces.

---

## Implementation Plan

### Phase 1: Add Workspace Detection Command

- [ ] Add new CLI subcommand `forge workspace is-indexed [PATH]` that checks if a path (or any ancestor) is already registered as a workspace
- [ ] Implement in `crates/forge_main/src/cli.rs` - add `IsIndexed` variant to `WorkspaceCommand` enum
- [ ] Add handler in `crates/forge_main/src/ui.rs` that calls existing `is_indexed()` service method
- [ ] The command should return exit code 0 if indexed, exit code 1 if not indexed
- [ ] Output should be silent by default (only exit code matters for scripting)
- [ ] Add optional `--verbose` flag to show which workspace was found (exact or ancestor)

### Phase 2: Update Zsh Background Sync Logic

- [ ] Modify `_forge_start_background_sync()` in `shell-plugin/lib/helpers.zsh:114-131`
- [ ] Before running sync, call `$_FORGE_BIN workspace is-indexed "$workspace_path"`
- [ ] Check exit code: if non-zero (not indexed), skip the sync and return early
- [ ] Only proceed with background sync if exit code is 0 (workspace or ancestor is already indexed)
- [ ] Add debug logging (when `FORGE_DEBUG=true`) to indicate when sync is skipped

### Phase 3: Handle Initial Workspace Registration

- [ ] Ensure manual `forge workspace sync` still works for first-time registration
- [ ] Update documentation to clarify the auto-sync behavior (only syncs already-registered workspaces)
- [ ] Consider adding a helper message when users first cd into a new directory that suggests running `forge workspace sync` to enable auto-sync

### Phase 4: Update Workspace List Display

- [ ] Enhance `on_list_workspaces()` in `crates/forge_main/src/ui.rs:3266-3311` to distinguish between exact match and ancestor match
- [ ] When current workspace is an ancestor match, display as `[Current via ancestor]` or similar
- [ ] Show the actual current directory path in the output when it differs from the workspace path

---

## Verification Criteria

### Before Fix
- [ ] Run `cd /Users/username` (parent directory)
- [ ] Run `forge workspace sync` once
- [ ] Run `cd /Users/username/Documents/Projects/forge` (subdirectory)
- [ ] Trigger zsh accept-line (press Enter on any command)
- [ ] Run `forge workspace list`
- [ ] Verify: `/Users/username/Documents/Projects/forge` is incorrectly registered as a workspace

### After Fix
- [ ] Clean database and repeat above steps
- [ ] Run `cd /Users/username/Documents/Projects/forge`
- [ ] Trigger zsh accept-line (press Enter)
- [ ] Run `forge workspace list`
- [ ] Verify: Only `/Users/username` appears (no auto-sync of subdirectory)
- [ ] Verify: List shows `[Current via ancestor]` or similar indication
- [ ] Run explicit `forge workspace sync` in subdirectory
- [ ] Verify: Now subdirectory is registered and will auto-sync on future visits

### Edge Cases
- [ ] Test behavior when no workspace exists at all (should not auto-sync)
- [ ] Test behavior in deeply nested directories (should find closest ancestor)
- [ ] Test behavior when `FORGE_SYNC_ENABLED=false` (should still respect flag)
- [ ] Test manual sync in new directory (should still work)

---

## Potential Risks and Mitigations

### Risk 1: Breaking Existing User Workflows
**Mitigation:** Users who rely on automatic workspace creation will need to run `forge workspace sync` once per workspace. This is a one-time migration cost for better UX long-term.

### Risk 2: Performance Impact
**Mitigation:** The `workspace is-indexed` check is a fast database query (already exists in `is_indexed()` method). It should add negligible overhead.

### Risk 3: Race Conditions
**Mitigation:** The check and sync are not atomic, but this is acceptable - worst case, a sync happens when it shouldn't have. No data corruption risk.

---

## Alternative Approaches

### Alternative 1: Opt-in Auto-Sync
Instead of checking if workspace exists, require users to explicitly enable auto-sync per workspace via a flag or configuration file.

**Pros:** More explicit control  
**Cons:** More complex UX, requires additional configuration management

### Alternative 2: Display-Only Fix
Just improve the workspace list display to show `[Current via ancestor]` without changing auto-sync behavior.

**Pros:** Simpler implementation, no behavior changes  
**Cons:** Doesn't solve the root problem of unintended workspace registrations

### Alternative 3: Auto-Sync with Depth Limit
Only auto-sync directories within N levels of an existing workspace.

**Pros:** Prevents deep directory pollution  
**Cons:** Arbitrary limit, still creates unintended workspaces

---

## Dependencies

- No external dependencies
- Existing `is_indexed()` method in `WorkspaceService` can be reused
- Zsh plugin already has access to `$_FORGE_BIN` for CLI calls

---

## Testing Strategy

1. **Unit Tests:** Add tests for `workspace is-indexed` command in CLI tests
2. **Integration Tests:** Test zsh function behavior with mocked CLI commands
3. **Manual Testing:** Follow verification criteria above
4. **Regression Testing:** Ensure existing workspace sync functionality unchanged

---

## Documentation Updates

- [ ] Update `shell-plugin/README.md` to explain auto-sync behavior
- [ ] Add note about one-time `workspace sync` requirement for new workspaces
- [ ] Document `FORGE_SYNC_ENABLED` environment variable
- [ ] Update workspace command documentation with new `is-indexed` subcommand
