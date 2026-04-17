# ACP Integration Tracking

> Personal reminder for when ForgeCode ACP support lands.

## Upstream References

- **Feature request**: [tailcallhq/forgecode#2968](https://github.com/tailcallhq/forgecode/issues/2968) — ACP support
- **PR (open)**: [tailcallhq/forgecode#2858](https://github.com/tailcallhq/forgecode/pull/2858) — Machine stdio transport for ACP
- **PR (draft)**: [tailcallhq/forgecode#2371](https://github.com/tailcallhq/forgecode/pull/2371) — ACP phase 1 testing
- **ACP spec**: https://zed.dev/acp

## Action Items

- [ ] Watch/subscribe to the above PRs and issue on GitHub
- [ ] When ACP PRs merge into upstream `main`:
  - [ ] Fetch and merge upstream into this fork's `main`
  - [ ] Rebuild local Forge binary: `cd /Volumes/990Pro2TB/OtherProjects/forgecode-fork && cargo build`
  - [ ] Test ACP transport: `forge --acp` or equivalent flag
- [ ] Once ForgeCode ACP is stable, test with Zed:
  - [ ] Verify ForgeCode appears in Zed's agent panel
  - [ ] End-to-end test: open a ForgeCode session from Zed via ACP
- [ ] Close/remove this tracking file once ACP is fully integrated and working locally

## Related

- Zed `--wait` bug (filed): [zed-industries/zed#54203](https://github.com/zed-industries/zed/issues/54203)

## Context

ACP (Agent Client Protocol) enables ForgeCode to run as an agent inside Zed and other ACP-compatible editors.
This completes the **ForgeCode + Warp + Zed** workflow by letting Zed invoke ForgeCode directly.