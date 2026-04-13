#!/usr/bin/env bash
# scripts/test-reasoning.sh
#
# Validates that reasoning parameters are correctly serialized for each
# provider across all supported effort levels.
#
# Usage: ./scripts/test-reasoning.sh

set -uo pipefail

# ─── colors ───────────────────────────────────────────────────────────────────

BOLD='\033[1m'
RESET='\033[0m'
GREEN='\033[32m'
RED='\033[31m'
YELLOW='\033[33m'
CYAN='\033[36m'
DIM='\033[2m'

# ─── state ────────────────────────────────────────────────────────────────────

PASS=0
FAIL=0
SKIP=0
BINARY="target/debug/forge"
WORK_DIR="$(mktemp -d)"
SEQ=0
RESULT_FILES=()
CURRENT_RF=""

cleanup() { rm -rf "$WORK_DIR"; }
trap cleanup EXIT

# ─── output helpers ───────────────────────────────────────────────────────────
# Each helper writes a tagged line to stdout.  Within a background subshell,
# stdout is redirected to a per-job result file; the main process reads it back
# after wait to tally counts and emit colour output in the original order.

log_header() { printf "HEADER\t%s\n" "$1"; }
log_pass()   { printf "PASS\t%s\n"   "$1"; }
log_fail()   { printf "FAIL\t%s\n"   "$1"; }
log_skip()   { printf "SKIP\t%s\n"   "$1"; }

# ─── json helpers ─────────────────────────────────────────────────────────────

# json_get <file> <dot.separated.path>
# Prints the JSON value at the given path, or "null" if absent/null.
# Uses raw_decode to parse only the first JSON object in the file, which
# correctly handles both single-document JSON and NDJSON (even when multiple
# objects appear on the same line without a newline separator).
json_get() {
    python3 - "$1" "$2" <<'PY'
import json, sys
with open(sys.argv[1]) as f:
    raw = f.read().strip()
# raw_decode stops after the first complete JSON value regardless of trailing
# content (extra objects, newlines, null bytes, etc.).
decoder = json.JSONDecoder()
d, _ = decoder.raw_decode(raw)
keys = sys.argv[2].split('.')
v = d
for k in keys:
    v = v.get(k) if isinstance(v, dict) else None
    if v is None:
        break
print(json.dumps(v))
PY
}

# assert_field <file> <dot.path> <expected_json_value> <label>
assert_field() {
    local file="$1" path="$2" expected="$3" label="$4"
    local actual
    actual="$(json_get "$file" "$path")"
    if [ "$actual" = "$expected" ]; then
        log_pass "$label  $path = $expected"
    else
        log_fail "$label  $path — expected $expected, got $actual"
    fi
}

# ─── test runner ──────────────────────────────────────────────────────────────

# run_test <outfile> <provider_id> <model_id> [KEY=VALUE ...]
# Runs forge with FORGE_DEBUG_REQUESTS pointing to <outfile>.
# Extra KEY=VALUE arguments are forwarded as additional env vars.
# Returns 0 if the request file was written, 1 otherwise (e.g. auth missing).
run_test() {
    local out="$1" provider="$2" model="$3"
    shift 3

    env FORGE_DEBUG_REQUESTS="$out" \
        FORGE_SESSION__PROVIDER_ID="$provider" \
        FORGE_SESSION__MODEL_ID="$model" \
        "$@" \
        "$BINARY" -p "Hello!" >/dev/null 2>&1 || true

    [ -f "$out" ]
}

# run_test_expect_failure <outfile> <provider_id> <model_id> [KEY=VALUE ...]
# Like run_test, but expects forge to exit non-zero and NOT write the request file.
# Invalid config values (e.g. unknown Effort variant) are rejected at startup,
# before any provider interaction, so this check is independent of credentials.
# Returns 0 if forge exited non-zero and wrote no file, 1 otherwise.
run_test_expect_failure() {
    local out="$1" provider="$2" model="$3"
    shift 3

    env FORGE_DEBUG_REQUESTS="$out" \
        FORGE_SESSION__PROVIDER_ID="$provider" \
        FORGE_SESSION__MODEL_ID="$model" \
        "$@" \
        "$BINARY" -p "Hello!" >/dev/null 2>&1
    local status=$?

    [ "$status" -ne 0 ] && [ ! -f "$out" ]
}

# ─── parallel job launcher ────────────────────────────────────────────────────

# next_result_file — allocates the next ordered result file path, appends it to
# RESULT_FILES, increments SEQ, and stores the path in CURRENT_RF.
# Must be called in the main process (NOT inside $()) so that RESULT_FILES and
# SEQ are updated in the parent shell.
next_result_file() {
    CURRENT_RF="$WORK_DIR/result-$(printf '%05d' "$SEQ").txt"
    RESULT_FILES+=("$CURRENT_RF")
    SEQ=$((SEQ + 1))
}

# ─── build ────────────────────────────────────────────────────────────────────

printf "${BOLD}Reasoning Serialization Tests${RESET}\n"
printf "${DIM}Building forge (debug)...${RESET}\n\n"
if ! cargo build 2>&1 | grep -E "^error|Finished|^   Compiling forge_main"; then
    printf "${RED}Build failed — aborting.${RESET}\n"
    exit 1
fi

# ─── OpenRouter · openai/o4-mini — effort levels ─────────────────────────────
# OpenRouter passes reasoning.effort straight through.
# Valid effort values: none · minimal · low · medium · high · xhigh
# Note: the default forge config sets reasoning.enabled=true; it always appears
# alongside any explicit effort. max_tokens and exclude remain absent.
# Ref: https://openrouter.ai/docs/guides/best-practices/reasoning-tokens

for effort in none minimal low medium high xhigh; do
    next_result_file
    (
        log_header "OpenRouter · openai/o4-mini · effort: $effort"
        OUT="$WORK_DIR/openrouter-openai-effort-$effort.json"
        if run_test "$OUT" open_router "openai/o4-mini" "FORGE_REASONING__EFFORT=$effort"; then
            assert_field "$OUT" "reasoning.effort"     "\"$effort\"" "openrouter/openai"
            assert_field "$OUT" "reasoning.max_tokens" "null"        "openrouter/openai"
            assert_field "$OUT" "reasoning.exclude"    "null"        "openrouter/openai"
        else
            log_skip "open_router not configured — skipping"
        fi
    ) > "$CURRENT_RF" &
done

# ─── OpenRouter · openai/o4-mini — max_tokens ────────────────────────────────
# When max_tokens is set, reasoning.max_tokens should appear.
# Note: the default forge config also injects effort="medium" and enabled=true;
# only max_tokens itself is verified here.

next_result_file
(
    log_header "OpenRouter · openai/o4-mini · max_tokens: 4000"
    OUT="$WORK_DIR/openrouter-openai-max-tokens.json"
    if run_test "$OUT" open_router "openai/o4-mini" FORGE_REASONING__MAX_TOKENS=4000; then
        assert_field "$OUT" "reasoning.max_tokens" "4000" "openrouter/openai"
    else
        log_skip "open_router not configured — skipping"
    fi
) > "$CURRENT_RF" &

# ─── OpenRouter · openai/o4-mini — exclude ───────────────────────────────────
# When exclude=true, reasoning runs internally but is omitted from the response.

next_result_file
(
    log_header "OpenRouter · openai/o4-mini · effort: high + exclude: true"
    OUT="$WORK_DIR/openrouter-openai-exclude.json"
    if run_test "$OUT" open_router "openai/o4-mini" \
            FORGE_REASONING__EFFORT=high FORGE_REASONING__EXCLUDE=true; then
        assert_field "$OUT" "reasoning.effort"  '"high"' "openrouter/openai"
        assert_field "$OUT" "reasoning.exclude" "true"   "openrouter/openai"
    else
        log_skip "open_router not configured — skipping"
    fi
) > "$CURRENT_RF" &

# ─── OpenRouter · openai/o4-mini — enabled ───────────────────────────────────
# enabled=true activates reasoning. The default config already sets enabled=true
# and effort="medium"; this test confirms enabled is explicitly passed through.

next_result_file
(
    log_header "OpenRouter · openai/o4-mini · enabled: true"
    OUT="$WORK_DIR/openrouter-openai-enabled.json"
    if run_test "$OUT" open_router "openai/o4-mini" FORGE_REASONING__ENABLED=true; then
        assert_field "$OUT" "reasoning.enabled" "true" "openrouter/openai"
        assert_field "$OUT" "reasoning.exclude" "null" "openrouter/openai"
    else
        log_skip "open_router not configured — skipping"
    fi
) > "$CURRENT_RF" &

# ─── OpenRouter · anthropic/claude-opus-4-5 — max_tokens ─────────────────────
# For Anthropic models via OpenRouter, max_tokens maps to budget_tokens.
# Valid range: integer >= 1024
# Note: default config injects effort="medium" and enabled=true alongside max_tokens.

next_result_file
(
    log_header "OpenRouter · anthropic/claude-opus-4-5 · max_tokens: 4000"
    OUT="$WORK_DIR/openrouter-anthropic-max-tokens.json"
    if run_test "$OUT" open_router "anthropic/claude-opus-4-5" FORGE_REASONING__MAX_TOKENS=4000; then
        assert_field "$OUT" "reasoning.max_tokens" "4000" "openrouter/anthropic"
    else
        log_skip "open_router not configured — skipping"
    fi
) > "$CURRENT_RF" &

# ─── OpenRouter · moonshotai/kimi-k2 — max_tokens ────────────────────────────
# Kimi K2 uses token-budget reasoning via OpenRouter (reasoning.max_tokens).
# Valid range: integer >= 1024

next_result_file
(
    log_header "OpenRouter · moonshotai/kimi-k2 · max_tokens: 4000"
    OUT="$WORK_DIR/openrouter-kimi-max-tokens.json"
    if run_test "$OUT" open_router "moonshotai/kimi-k2" FORGE_REASONING__MAX_TOKENS=4000; then
        assert_field "$OUT" "reasoning.max_tokens" "4000" "openrouter/kimi-k2"
    else
        log_skip "open_router not configured — skipping"
    fi
) > "$CURRENT_RF" &

next_result_file
(
    log_header "OpenRouter · moonshotai/kimi-k2 · effort: high"
    OUT="$WORK_DIR/openrouter-kimi-effort-high.json"
    if run_test "$OUT" open_router "moonshotai/kimi-k2" FORGE_REASONING__EFFORT=high; then
        assert_field "$OUT" "reasoning.effort" '"high"' "openrouter/kimi-k2"
    else
        log_skip "open_router not configured — skipping"
    fi
) > "$CURRENT_RF" &

# ─── OpenRouter · minimax/minimax-m2 — max_tokens ────────────────────────────
# MiniMax M2 uses token-budget reasoning via OpenRouter; maps to thinking_budget.
# Valid range: integer >= 1024

next_result_file
(
    log_header "OpenRouter · minimax/minimax-m2 · max_tokens: 4000"
    OUT="$WORK_DIR/openrouter-minimax-max-tokens.json"
    if run_test "$OUT" open_router "minimax/minimax-m2" FORGE_REASONING__MAX_TOKENS=4000; then
        assert_field "$OUT" "reasoning.max_tokens" "4000" "openrouter/minimax-m2"
    else
        log_skip "open_router not configured — skipping"
    fi
) > "$CURRENT_RF" &

next_result_file
(
    log_header "OpenRouter · minimax/minimax-m2 · effort: high"
    OUT="$WORK_DIR/openrouter-minimax-effort-high.json"
    if run_test "$OUT" open_router "minimax/minimax-m2" FORGE_REASONING__EFFORT=high; then
        assert_field "$OUT" "reasoning.effort" '"high"' "openrouter/minimax-m2"
    else
        log_skip "open_router not configured — skipping"
    fi
) > "$CURRENT_RF" &

# ─── Anthropic · claude-opus-4-6 — effort levels ─────────────────────────────
# Newer models use output_config.effort instead of the thinking object.
# Valid effort values: low · medium · high · max  (max is opus-4-6 only)
# Ref: https://platform.claude.com/docs/en/build-with-claude/effort

for effort in low medium high max; do
    next_result_file
    (
        log_header "Anthropic · claude-opus-4-6 · effort: $effort"
        OUT="$WORK_DIR/anthropic-opus46-effort-$effort.json"
        if run_test "$OUT" anthropic "claude-opus-4-6" "FORGE_REASONING__EFFORT=$effort"; then
            assert_field "$OUT" "output_config.effort" "\"$effort\"" "anthropic/opus4.6"
            assert_field "$OUT" "thinking"             "null"        "anthropic/opus4.6"
        else
            log_skip "anthropic not configured — skipping"
        fi
    ) > "$CURRENT_RF" &
done

# ─── Anthropic · claude-3-7-sonnet-20250219 — thinking object ────────────────
# Older models use the thinking object with budget_tokens instead of effort.
# budget_tokens must be > 1024 and < max_tokens.
# Ref: https://platform.claude.com/docs/en/build-with-claude/effort

next_result_file
(
    log_header "Anthropic · claude-3-7-sonnet-20250219 · enabled: true + max_tokens: 8000"
    OUT="$WORK_DIR/anthropic-sonnet37-thinking.json"
    if run_test "$OUT" anthropic "claude-3-7-sonnet-20250219" \
            FORGE_REASONING__ENABLED=true FORGE_REASONING__MAX_TOKENS=8000; then
        assert_field "$OUT" "thinking.type"          '"enabled"' "anthropic/sonnet3.7"
        assert_field "$OUT" "thinking.budget_tokens" "8000"      "anthropic/sonnet3.7"
        assert_field "$OUT" "output_config"          "null"      "anthropic/sonnet3.7"
    else
        log_skip "anthropic not configured — skipping"
    fi
) > "$CURRENT_RF" &

# ─── GitHub Copilot · o4-mini — effort levels ────────────────────────────────
# Chat Completions API serializes reasoning as a top-level reasoning_effort string.
# Valid effort values: none · minimal · low · medium · high · xhigh
# Ref: https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create

for effort in none minimal low medium high xhigh; do
    next_result_file
    (
        log_header "GitHub Copilot · o4-mini · effort: $effort"
        OUT="$WORK_DIR/github-copilot-effort-$effort.json"
        if run_test "$OUT" github_copilot "o4-mini" "FORGE_REASONING__EFFORT=$effort"; then
            assert_field "$OUT" "reasoning_effort" "\"$effort\"" "github_copilot/o4-mini"
            assert_field "$OUT" "reasoning"        "null"        "github_copilot/o4-mini"
        else
            log_skip "github_copilot not configured — skipping"
        fi
    ) > "$CURRENT_RF" &
done

# ─── Codex · gpt-5.1-codex — effort levels ───────────────────────────────────
# Responses API uses a nested reasoning object with effort + summary fields.
# Each effort value is passed through as-is; summary defaults to "auto".
# Ref: https://developers.openai.com/api/docs/guides/reasoning

for effort in none minimal low medium high xhigh; do
    next_result_file
    (
        log_header "Codex · gpt-5.1-codex · effort: $effort"
        OUT="$WORK_DIR/codex-effort-$effort.json"
        if run_test "$OUT" codex "gpt-5.1-codex" "FORGE_REASONING__EFFORT=$effort"; then
            assert_field "$OUT" "reasoning.effort"  "\"$effort\""  "codex/gpt-5.1-codex"
            assert_field "$OUT" "reasoning.summary" '"auto"'       "codex/gpt-5.1-codex"
            assert_field "$OUT" "reasoning_effort"  "null"         "codex/gpt-5.1-codex"
        else
            log_skip "codex not configured — skipping"
        fi
    ) > "$CURRENT_RF" &
done

# ─── Codex · gpt-5.1-codex — exclude ────────────────────────────────────────
# When exclude=true the effort is passed through unchanged and summary="concise".

next_result_file
(
    log_header "Codex · gpt-5.1-codex · effort: medium + exclude: true"
    OUT="$WORK_DIR/codex-exclude.json"
    if run_test "$OUT" codex "gpt-5.1-codex" \
            FORGE_REASONING__EFFORT=medium FORGE_REASONING__EXCLUDE=true; then
        assert_field "$OUT" "reasoning.effort"  '"medium"'    "codex/gpt-5.1-codex"
        assert_field "$OUT" "reasoning.summary" '"concise"'   "codex/gpt-5.1-codex"
    else
        log_skip "codex not configured — skipping"
    fi
) > "$CURRENT_RF" &

# ─── Invalid effort — config parse error (one per provider) ──────────────────
# "invalid" is not a recognised Effort variant. Forge must reject it at config
# parse time, exit non-zero, and never write a debug request file.
# This check runs regardless of provider credentials.

next_result_file
(
    log_header "Invalid effort · config parse error"
    for entry in \
        "open_router:openai/o4-mini" \
        "anthropic:claude-opus-4-6" \
        "github_copilot:o4-mini" \
        "codex:gpt-5.1-codex"
    do
        provider="${entry%%:*}"
        model="${entry##*:}"
        OUT="$WORK_DIR/invalid-effort-${provider}.json"
        if run_test_expect_failure "$OUT" "$provider" "$model" FORGE_REASONING__EFFORT=invalid; then
            log_pass "$provider/$model  invalid effort → non-zero exit, no request written"
        else
            log_fail "$provider/$model  invalid effort was not rejected"
        fi
    done
) > "$CURRENT_RF" &

# ─── collect results ──────────────────────────────────────────────────────────

wait  # wait for all background jobs to finish

for f in "${RESULT_FILES[@]}"; do
    [ -f "$f" ] || continue
    while IFS=$'\t' read -r type msg; do
        case "$type" in
            HEADER) printf "\n${BOLD}${CYAN}▶  %s${RESET}\n" "$msg" ;;
            PASS)   printf "  ${GREEN}✓${RESET}  %s\n" "$msg"; PASS=$((PASS + 1)) ;;
            FAIL)   printf "  ${RED}✗${RESET}  %s\n" "$msg"; FAIL=$((FAIL + 1)) ;;
            SKIP)   printf "  ${YELLOW}~${RESET}  %s\n" "$msg"; SKIP=$((SKIP + 1)) ;;
        esac
    done < "$f"
done

# ─── summary ──────────────────────────────────────────────────────────────────

printf "\n${BOLD}─────────────────────────────────────────${RESET}\n"
printf "${BOLD}Results:${RESET}  "
printf "${GREEN}%d passed${RESET}  " "$PASS"
[ "$FAIL" -gt 0 ] && printf "${RED}%d failed${RESET}  " "$FAIL" \
                  || printf "${DIM}%d failed${RESET}  " "$FAIL"
printf "${YELLOW}%d skipped${RESET}\n\n" "$SKIP"

[ "$FAIL" -eq 0 ]
