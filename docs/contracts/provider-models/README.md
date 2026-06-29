# Provider-Model Contract

**Version:** 1.0.0
**Status:** Reference artifact — not yet a published package.

## Purpose

This directory contains the language-agnostic contract for the provider/model registry
surface shared across three KooshaPari repos that independently implement the same domain:

| Repo | Language | Role |
|------|----------|------|
| forgecode | Rust | CLI coding agent — reference implementation |
| OmniRoute | TypeScript | LLM router / proxy |
| cliproxy | Go | CLI auth proxy |

All three implement `Provider → Models → Capabilities` and the same SSE/OAuth stop rules.
Rather than a single shared binary (impossible across Rust/TS/Go without FFI/WASM overhead),
the contract is a **JSON Schema** that each repo aligns its native types against.

## Files

| File | Description |
|------|-------------|
| `provider-model.schema.json` | JSON Schema 2020-12 for `Model`, `ProviderConfig`, `SseStopRule`, `OAuthRefreshPolicy` |
| `README.md` | This file |

## How to use this contract

### forgecode (Rust)

`forge_domain::Model` and `forge_domain::Provider` are the reference implementation.
`forge_eventsource::is_sse_terminal` is the reference implementation of `SseStopRule`.
When the domain types change, update the schema to stay in sync.

### OmniRoute (TypeScript)

Optionally codegen TypeScript types via:

```bash
npx json-schema-to-typescript provider-model.schema.json -o src/types/provider-model.d.ts
```

Align `src/lib/modelCapabilities.ts`, `src/lib/sseTextTransform.ts`, and
`src/lib/tokenHealthCheck.ts` against the schema semantics (field names, enum values,
`is_sse_terminal` logic, and `TOKEN_EXPIRY_BUFFER`).

### cliproxy (Go)

Optionally codegen Go structs via:

```bash
go-jsonschema -p registry provider-model.schema.json -o pkg/llmproxy/registry/provider_model_gen.go
```

Align `pkg/llmproxy/registry/model_registry.go` field names and capability enums to the schema.
Per-provider `RefreshLead()` overrides (e.g. codebuddy 24 h) are valid because the
`oauth_refresh_policy.default_refresh_lead_seconds` field is explicitly *parameterized*.

## SSE terminal-marker rules (normative)

Implementations MUST treat the following SSE event data values as end-of-stream:

- `[DONE]` — the canonical OpenAI/Anthropic sentinel
- `""` (empty string) — keepalive / implicit close

Additionally:
- OpenAI: `choices[0].finish_reason` in `{stop, length, content_filter, tool_calls}` signals model completion.
- Anthropic: `stop_reason` / `message_delta.stop_reason` fields signal model completion.
- **Synthetic `[DONE]` on silent close:** when the upstream connection closes without an
  explicit terminal event, implementations MUST emit a synthetic terminal signal rather
  than propagating an unexpected EOF to callers.

See `forge_eventsource::is_sse_terminal` for the canonical Rust implementation.

## OAuth refresh policy (normative)

A token needs refresh when:

```
now + refresh_lead >= token.expires_at
```

Default `refresh_lead` is **300 seconds (5 minutes)**, matching:
- forgecode: `chrono::Duration::minutes(5)`
- OmniRoute: `TOKEN_EXPIRY_BUFFER = 5 * 60 * 1000`
- cliproxy: `5 * time.Minute` (most providers)

Per-provider overrides are valid (e.g. cliproxy codebuddy uses 86400 s).
The contract requires the lead to be *parameterized*, not hardcoded.

## Versioning

Contract changes follow semver:
- **Patch** — clarifications, description-only updates, no field changes.
- **Minor** — new optional fields; existing fields unchanged.
- **Major** — field renames, type changes, or removal of fields.

Each consuming repo should record the contract version it was aligned against in its
own changelog or a `docs/contracts/VERSION` pin file.
