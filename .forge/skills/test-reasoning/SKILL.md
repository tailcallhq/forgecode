---
name: test-reasoning
description: Validate that reasoning parameters are correctly serialized and sent to provider APIs. Use when the user asks to test reasoning serialization, run reasoning tests, verify reasoning config fields, or check that ReasoningConfig maps correctly to provider-specific JSON (OpenRouter, Anthropic, GitHub Copilot, Codex).
---

# Test Reasoning Serialization

Validates that `ReasoningConfig` fields are correctly serialized into provider-specific JSON
for OpenRouter, Anthropic, GitHub Copilot, and Codex.

## Quick Start

Run all tests with the bundled script:

```bash
./scripts/test-reasoning.sh
```

The script builds forge in debug mode, runs each provider/model combination, captures the
outgoing HTTP request body via `FORGE_DEBUG_REQUESTS`, and asserts the correct JSON fields.

## Running a Single Test Manually

```bash
FORGE_DEBUG_REQUESTS="forge.request.json" \
FORGE_SESSION__PROVIDER_ID=<provider_id> \
FORGE_SESSION__MODEL_ID=<model_id> \
FORGE_REASONING__EFFORT=<effort> \
target/debug/forge -p "Hello!"
```

Then inspect `.forge/forge.request.json` for the expected fields.

## Test Coverage

| Provider         | Model                        | Config fields                                     | Expected JSON field               |
| ---------------- | ---------------------------- | ------------------------------------------------- | --------------------------------- |
| `open_router`    | `openai/o4-mini`             | `effort: none\|minimal\|low\|medium\|high\|xhigh` | `reasoning.effort`                |
| `open_router`    | `openai/o4-mini`             | `max_tokens: 4000`                                | `reasoning.max_tokens`            |
| `open_router`    | `openai/o4-mini`             | `effort: high` + `exclude: true`                  | `reasoning.effort` + `.exclude`   |
| `open_router`    | `openai/o4-mini`             | `enabled: true`                                   | `reasoning.enabled`               |
| `open_router`    | `anthropic/claude-opus-4-5`  | `max_tokens: 4000`                                | `reasoning.max_tokens`            |
| `open_router`    | `moonshotai/kimi-k2`         | `max_tokens: 4000`                                | `reasoning.max_tokens`            |
| `open_router`    | `moonshotai/kimi-k2`         | `effort: high`                                    | `reasoning.effort`                |
| `open_router`    | `minimax/minimax-m2`         | `max_tokens: 4000`                                | `reasoning.max_tokens`            |
| `open_router`    | `minimax/minimax-m2`         | `effort: high`                                    | `reasoning.effort`                |
| `anthropic`      | `claude-opus-4-6`            | `effort: low\|medium\|high\|max`                  | `output_config.effort`            |
| `anthropic`      | `claude-3-7-sonnet-20250219` | `enabled: true` + `max_tokens: 8000`              | `thinking.type` + `budget_tokens` |
| `github_copilot` | `o4-mini`                    | `effort: none\|minimal\|low\|medium\|high\|xhigh` | `reasoning_effort` (top-level)    |
| `codex`          | `gpt-5.1-codex`              | `effort: none\|minimal\|low\|medium\|high\|xhigh` | `reasoning.effort` + `.summary`   |
| `codex`          | `gpt-5.1-codex`              | `effort: medium` + `exclude: true`                | `reasoning.summary = "concise"`   |
| all providers    | one model each               | `effort: invalid`                                 | non-zero exit, no request written |

Tests for unconfigured providers are skipped automatically. Invalid-effort tests run regardless of credentials — the rejection happens at config parse time before any provider interaction.

## References

- [OpenAI Reasoning guide](https://developers.openai.com/api/docs/guides/reasoning)
- [OpenAI Chat Completions API reference](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create)
- [Anthropic Extended Thinking](https://platform.claude.com/docs/en/build-with-claude/effort)
- [OpenRouter Reasoning Tokens](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens)
