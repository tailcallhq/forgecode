## Summary

<!-- What does this PR do, and why? One or two sentences. Link issues if any. -->

## Linked FR

<!-- Which functional requirement does this change implement or affect?
     Reference the ID(s) from FUNCTIONAL_REQUIREMENTS.md (e.g. FR-002, FR-014).
     Add "N/A — no FR covers this change" only if truly none applies. -->

- [ ] FR-001 Core agentic loop
- [ ] FR-002 Tool execution
- [ ] FR-003 TUI & keyboard interaction
- [ ] FR-004 Configuration
- [ ] FR-005 ZSH plugin
- [ ] FR-006 Eval harness
- [ ] FR-007 Conversation persistence
- [ ] FR-008 MCP client
- [ ] FR-009 Providers & authentication
- [ ] FR-010 Self-update
- [ ] FR-011 Distribution & install
- [ ] FR-012 Semantic search & workspaces
- [ ] FR-013 Commit generation
- [ ] FR-014 Accessibility & screen-reader mode
- [ ] FR-015 Golden-output tests
- [ ] N/A

## Tests run

<!-- List the exact commands you ran and their outcome. Copy from your shell. -->

```bash
cargo check -p <crate>
cargo test -p <crate>            # or: cargo nextest run -p <crate>
cargo insta test --accept        # only when snapshot changes are intended
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
npm run eval <task.yml>          # for eval-harness changes
```

- [ ] `cargo check` passes for every touched crate
- [ ] Tests pass for touched crates
- [ ] Clippy `-D warnings` passes on touched crates
- [ ] `cargo fmt --check` passes

## Screenshots / TUI output

<!-- Required for UI, theming, prompt, banner, or ZSH-plugin changes.
     Paste terminal output or link a screenshot/gif. Otherwise write "N/A". -->

## Checklist

- [ ] Docs updated (README, docs/, FUNCTIONAL_REQUIREMENTS.md, VISUAL_SPEC.md as applicable)
- [ ] No secrets or credentials introduced (rely on env vars / `~/.forge` credential store)
- [ ] Rust docs (`///`) present on all new public items
- [ ] Tests follow AGENTS.md conventions (pretty_assertions, fixture/actual/expected)
- [ ] CI gates pass (ci / lint / test / cargo-deny / trufflehog)

## Risk & rollback

<!-- One-liner: worst-case impact if this merges and misbehaves, and how to revert. -->
