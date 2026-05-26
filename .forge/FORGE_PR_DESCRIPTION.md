## Summary
Add a permission case system with required caller context and paged evidence display for write/patch/shell operations in restricted mode.

## Context
When using forge in restricted mode, the permission approval flow had three problems:

1. **No context at decision time**: The TUI permission prompt cleared the terminal, erasing all context about what was being approved
2. **No caller justification**: Write/patch operations had no way to carry the LLM's reasoning or justification through to the permission prompt
3. **No evidence inspection**: Users had no way to inspect the full proposed diff before making a decision — only a truncated single-line message was visible

This implements a judicial-style evidence collection system where every decision point is preceded by a full case brief showing what changed, why, and where to find the full details.

## Changes

### Permission Case System (new: `crates/forge_domain/src/policies/case.rs`)
- `PermissionCase` struct collects all decision evidence: case_id, timestamp, operation type, file path, proposed changes, and caller explanation
- Atomic counter ensures unique case IDs across rapid sequential decisions
- `format_panel()` renders a styled evidence panel for display
- Tests verify case creation, ID uniqueness, panel rendering, and patch diff display

### Required `context` field on tool calls (`crates/forge_domain/src/tools/catalog.rs`)
- `FSWrite`, `FSPatch`, `FSMultiPatch` now require a `context: String` field
- The LLM must provide a justification for every write/patch (enforced via JSON schema, no `#[serde(default)]`)
- `Shell` also requires `context: String` for command reasoning
- `CommandType` enum classifies shell commands: `InlineCode` (here-docs, eval, embedded scripts — auto-denied), `FileScript`, `Utility`
- All snapshots and test fixtures updated for the new schema

### Paged evidence display (`crates/forge_app/src/tool_registry.rs`)
- `build_case()` constructs the full case from tool input, collecting untruncated content
- `print_case()` writes the case brief to `/tmp/forge-cases/<case_id>.md` and pipes it through `less` (respects `$PAGER` env)
- After the user quits the pager, the TUI permission prompt appears
- `check_tool_permission()` appends the case file path to the TUI message for later reference
- Inline-code shell commands are auto-denied before reaching the TUI
- Full untruncated diffs shown (removed 120-char truncation limits since content is now paged)

### Permission operation message field (`crates/forge_domain/src/policies/operation.rs`)
- `PermissionOperation::Execute` now carries a `message: String` for classification info
- All pattern matches updated (`policy.rs`, `rule.rs`, `engine.rs`)

### `fmt_input.rs` enhancements
- Write/Patch/MultiPatch tool input display now shows proposed content/diff inline

## Use Cases
- **Write approval**: See full file content in `less` before allowing the write
- **Patch approval**: Scroll through the complete old→new diff in the pager before deciding
- **Shell execution**: See the command type classification (auto-deny inline code)
- **Audit trail**: Every permission decision has a case file in `/tmp/forge-cases/` with full evidence

## Testing
```bash
# Run all relevant tests
cargo test -p forge_domain -p forge_app -p forge_services

# All 1532 tests pass, 0 failures

# To test interactively:
# 1. Start forge in restricted mode
# 2. Request a write or patch operation
# 3. The pager (less) should show the full evidence before the TUI prompt
# 4. Check /tmp/forge-cases/ for the persistent case file
```

## Links
- Based on discussion about judicial-style approval workflow with evidence collection and traceable decision IDs
