#!/usr/bin/env zsh

# Integration tests for FORGE_SESSION__* and FORGE_REASONING__* environment
# variable support in `forge config list`.
#
# These tests verify that env vars set by the shell plugin (e.g.
# FORGE_SESSION__MODEL_ID, FORGE_SESSION__PROVIDER_ID, FORGE_REASONING__EFFORT)
# are picked up and reflected in the resolved configuration output.
#
# Usage: zsh scripts/test-config-env.sh

set -euo pipefail

# ---------------------------------------------------------------------------
# Colours & counters
# ---------------------------------------------------------------------------
BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'
GREEN='\033[32m'
RED='\033[31m'
CYAN='\033[36m'

PASS=0
FAIL=0

# ---------------------------------------------------------------------------
# Resolve forge binary
# ---------------------------------------------------------------------------
SCRIPT_DIR="${0:A:h}"
FORGE_BIN="${FORGE_BIN:-${SCRIPT_DIR}/../target/debug/forge}"

if [[ ! -x "$FORGE_BIN" ]]; then
    echo "${RED}forge binary not found at ${FORGE_BIN}${RESET}"
    echo "Run: cargo build -p forge_main"
    exit 1
fi

# ---------------------------------------------------------------------------
# Test harness helpers
# ---------------------------------------------------------------------------

function assert_contains() {
    local test_name="$1"
    local haystack="$2"
    local needle="$3"

    if [[ "$haystack" == *"$needle"* ]]; then
        printf "  ${GREEN}✓${RESET} %s\n" "$test_name"
        PASS=$(( PASS + 1 ))
    else
        printf "  ${RED}✗${RESET} %s\n" "$test_name"
        printf "    ${DIM}expected to find:${RESET} %s\n" "$needle"
        printf "    ${DIM}in output:${RESET}\n%s\n" "$haystack"
        FAIL=$(( FAIL + 1 ))
    fi
}

function assert_not_contains() {
    local test_name="$1"
    local haystack="$2"
    local needle="$3"

    if [[ "$haystack" != *"$needle"* ]]; then
        printf "  ${GREEN}✓${RESET} %s\n" "$test_name"
        PASS=$(( PASS + 1 ))
    else
        printf "  ${RED}✗${RESET} %s\n" "$test_name"
        printf "    ${DIM}expected NOT to find:${RESET} %s\n" "$needle"
        printf "    ${DIM}in output:${RESET}\n%s\n" "$haystack"
        FAIL=$(( FAIL + 1 ))
    fi
}

# Run `forge config list --porcelain` with the given extra env vars prepended.
function config_list() {
    env "$@" "$FORGE_BIN" config list --porcelain 2>/dev/null
}

# ---------------------------------------------------------------------------
# Tests: FORGE_SESSION__* env vars appear in config list output
# ---------------------------------------------------------------------------
echo ""
echo -e "${BOLD}FORGE_SESSION__* env vars${RESET} ${DIM}— forge config list${RESET}"
echo ""

# When FORGE_SESSION__* are NOT set, no [session] block should appear.
OUTPUT="$(config_list)"
assert_not_contains \
    "no session block when env vars absent" \
    "$OUTPUT" \
    'model_id = "env-model"'

# When both are set, the resolved config must show the session.
OUTPUT="$(config_list \
    FORGE_SESSION__PROVIDER_ID=env-provider \
    FORGE_SESSION__MODEL_ID=env-model)"

assert_contains \
    "model_id reflects FORGE_SESSION__MODEL_ID" \
    "$OUTPUT" \
    'model_id = "env-model"'

assert_contains \
    "provider_id reflects FORGE_SESSION__PROVIDER_ID" \
    "$OUTPUT" \
    'provider_id = "env-provider"'

# Env var must override a different value that might be in the disk config.
# We simulate this by running once without and once with the env var and
# checking that the env var value wins.
OUTPUT_WITH_ENV="$(config_list \
    FORGE_SESSION__PROVIDER_ID=override-provider \
    FORGE_SESSION__MODEL_ID=override-model)"

assert_contains \
    "env var overrides any disk-stored session model" \
    "$OUTPUT_WITH_ENV" \
    'model_id = "override-model"'

assert_contains \
    "env var overrides any disk-stored session provider" \
    "$OUTPUT_WITH_ENV" \
    'provider_id = "override-provider"'

# ---------------------------------------------------------------------------
# Tests: FORGE_REASONING__EFFORT env var appears in config list output
# ---------------------------------------------------------------------------
echo ""
echo -e "${BOLD}FORGE_REASONING__EFFORT env var${RESET} ${DIM}— forge config list${RESET}"
echo ""

OUTPUT_REASONING="$(config_list FORGE_REASONING__EFFORT=high)"
assert_contains \
    "effort reflects FORGE_REASONING__EFFORT=high" \
    "$OUTPUT_REASONING" \
    'effort = "high"'

OUTPUT_REASONING_MIN="$(config_list FORGE_REASONING__EFFORT=minimal)"
assert_contains \
    "effort reflects FORGE_REASONING__EFFORT=minimal" \
    "$OUTPUT_REASONING_MIN" \
    'effort = "minimal"'

# ---------------------------------------------------------------------------
# Tests: _forge_action_config passes session env vars (ZSH plugin regression)
# ---------------------------------------------------------------------------
echo ""
echo -e "${BOLD}_forge_action_config${RESET} ${DIM}— session vars forwarded via _forge_exec${RESET}"
echo ""

# Write a small standalone zsh script that sources the plugin helpers and runs
# _forge_action_config with session vars set.  Using a separate script avoids
# set -euo pipefail interactions from the outer test harness.
_PLUGIN_SCRIPT=$(mktemp /tmp/forge_plugin_test_XXXXXX.zsh)
cat >"$_PLUGIN_SCRIPT" <<PLUGIN_TEST
#!/usr/bin/env zsh
_FORGE_BIN='${FORGE_BIN}'
_FORGE_ACTIVE_AGENT=forge
_FORGE_SESSION_MODEL=plugin-model
_FORGE_SESSION_PROVIDER=plugin-provider
_FORGE_SESSION_REASONING_EFFORT=
_FORGE_TERM=false
typeset -a _FORGE_TERM_COMMANDS=()
source '${SCRIPT_DIR}/../shell-plugin/lib/helpers.zsh'
source '${SCRIPT_DIR}/../shell-plugin/lib/actions/config.zsh'
_forge_action_config
PLUGIN_TEST

OUTPUT_PLUGIN="$(NO_COLOR=1 zsh "$_PLUGIN_SCRIPT" 2>/dev/null)"
echo "DEBUG: captured $(echo "$OUTPUT_PLUGIN" | wc -l) lines, script=$_PLUGIN_SCRIPT"
echo "DEBUG: grep result: $(echo "$OUTPUT_PLUGIN" | grep -o 'model_id.*plugin-model' | head -1)"
rm -f "$_PLUGIN_SCRIPT"

assert_contains \
    "_forge_action_config forwards FORGE_SESSION__MODEL_ID" \
    "$OUTPUT_PLUGIN" \
    'model_id = "plugin-model"'

assert_contains \
    "_forge_action_config forwards FORGE_SESSION__PROVIDER_ID" \
    "$OUTPUT_PLUGIN" \
    'provider_id = "plugin-provider"'

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
TOTAL=$(( PASS + FAIL ))
if (( FAIL > 0 )); then
    echo -e "${RED}${BOLD}FAILED${RESET} ${PASS}/${TOTAL} passed, ${FAIL} failed"
    echo ""
    exit 1
else
    echo -e "${GREEN}${BOLD}ALL PASSED${RESET} ${PASS}/${TOTAL}"
    echo ""
fi
