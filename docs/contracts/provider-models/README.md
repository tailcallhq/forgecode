# Provider-Model Contract

**Version:** 1.0.0
**Status:** Vendored pin — CANONICAL SSOT is `KooshaPari/phenotype-contracts`.

## Canonical Home

> **The authoritative source for these schemas is
> [KooshaPari/phenotype-contracts](https://github.com/KooshaPari/phenotype-contracts).**
>
> The copies in this directory (`docs/contracts/provider-models/`) are a **vendored pin**
> of that SSOT. Do not edit them here; open a PR against `phenotype-contracts` instead
> and then re-vendor the updated files.

**Pinned ref:** `cc8f34ed34a3f1ae2ba7edd6810a902e51738693`
(phenotype-contracts `main` HEAD at time of pin — 2026-06-28)

---

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
| `oauth-refresh-policy.schema.json` | JSON Schema 2020-12 for OAuth token refresh timing contract |
| `resilience-policy.schema.json` | JSON Schema 2020-12 for retry/backoff parameters and retryable-error taxonomy |
| `README.md` | This file |

## How to use this contract

### forgecode (Rust)

`forge_domain::Model` and `forge_domain::Provider` are the reference implementation.
`forge_eventsource::is_sse_terminal` is the reference implementation of `SseStopRule`.
When the domain types change, update the schema to stay in sync.

A conformance test in `crates/forge_eventsource/tests/contract_conformance.rs` asserts
that forgecode's runtime constants match the contract values declared in these schemas.
Run it with `cargo test contract_conformance`.

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
- forgecode: `OAUTH_REFRESH_LEAD = chrono::Duration::minutes(5)` (`forge_services::provider_auth`)
- OmniRoute: `TOKEN_EXPIRY_BUFFER = 5 * 60 * 1000`
- cliproxy: `5 * time.Minute` (most providers)

Per-provider overrides are valid (e.g. cliproxy codebuddy uses 86400 s).
The contract requires the lead to be *parameterized*, not hardcoded.

## Retryable HTTP status codes (normative)

The following HTTP status codes MUST trigger a retry (source: `resilience-policy.schema.json`):

`408, 429, 500, 502, 503, 504, 520, 522, 524, 529`

forgecode reference: `forge_config::RetryConfig` default `status_codes`.

## Re-vendoring

When `KooshaPari/phenotype-contracts` merges a schema change:

1. Copy the updated `*.schema.json` files here.
2. Update the **Pinned ref** SHA at the top of this README.
3. Run `cargo test contract_conformance` to verify forgecode's constants still match.
4. Commit with message `chore(contracts): re-vendor phenotype-contracts@<sha>`.

## Versioning

Contract changes follow semver:
- **Patch** — clarifications, description-only updates, no field changes.
- **Minor** — new optional fields; existing fields unchanged.
- **Major** — field renames, type changes, or removal of fields.
