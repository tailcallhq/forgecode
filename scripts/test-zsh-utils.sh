#!/usr/bin/env zsh

# Correctness and performance tests for `forge zsh format` which wraps bare
# file paths in @[...] syntax.  All parsing logic now lives in Rust; these
# tests exercise the CLI subcommand end-to-end.
#
# Usage: zsh scripts/test-zsh-utils.sh

set -euo pipefail
zmodload zsh/datetime

# Colors
BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'
GREEN='\033[32m'
RED='\033[31m'
CYAN='\033[36m'

PASS=0
FAIL=0

# Resolve the forge binary (prefer local debug build)
SCRIPT_DIR="${0:A:h}"
FORGE_BIN="${FORGE_BIN:-${SCRIPT_DIR}/../target/debug/forge}"

if [[ ! -x "$FORGE_BIN" ]]; then
    echo "${RED}forge binary not found at ${FORGE_BIN}${RESET}"
    echo "Run: cargo build -p forge_main"
    exit 1
fi

# Wrapper that calls the Rust formatter
function format() {
    "$FORGE_BIN" zsh format --buffer "$1"
}

# Create temporary files for testing paths with spaces
TMPDIR_TEST=$(mktemp -d)
mkdir -p "${TMPDIR_TEST}/my folder"
touch "${TMPDIR_TEST}/my folder/test file.txt"
touch "${TMPDIR_TEST}/simple.txt"

# --- Test harness -----------------------------------------------------------

function assert_eq() {
    local test_name="$1"
    local actual="$2"
    local expected="$3"

    if [[ "$actual" == "$expected" ]]; then
        printf "  ${GREEN}✓${RESET} %s\n" "$test_name"
        PASS=$(( PASS + 1 ))
    else
        printf "  ${RED}✗${RESET} %s\n" "$test_name"
        printf "    ${DIM}expected:${RESET} %s\n" "$expected"
        printf "    ${DIM}  actual:${RESET} %s\n" "$actual"
        FAIL=$(( FAIL + 1 ))
    fi
}

# --- Correctness tests ------------------------------------------------------

echo ""
echo -e "${BOLD}Correctness Tests${RESET} ${DIM}— forge zsh format${RESET}"
echo ""

# Basic wrapping
assert_eq "bare existing path" \
    "$(format "/usr/bin/env")" \
    "@[/usr/bin/env]"

assert_eq "path in sentence" \
    "$(format "look at /usr/bin/env please")" \
    "look at @[/usr/bin/env] please"

# Non-existent paths left untouched
assert_eq "nonexistent path untouched" \
    "$(format "check /nonexistent/foo.rs")" \
    "check /nonexistent/foo.rs"

# Already wrapped left untouched
assert_eq "already wrapped @[...] untouched" \
    "$(format "check @[/usr/bin/env] ok")" \
    "check @[/usr/bin/env] ok"

# Plain text (no paths)
assert_eq "plain text no paths" \
    "$(format "hello world")" \
    "hello world"

# Paths with spaces
assert_eq "bare path with spaces" \
    "$(format "${TMPDIR_TEST}/my folder/test file.txt")" \
    "@[${TMPDIR_TEST}/my folder/test file.txt]"

# Quoted paths with spaces
assert_eq "single-quoted path with spaces" \
    "$(format "'${TMPDIR_TEST}/my folder/test file.txt'")" \
    "@[${TMPDIR_TEST}/my folder/test file.txt]"

assert_eq "double-quoted path with spaces" \
    "$(format "\"${TMPDIR_TEST}/my folder/test file.txt\"")" \
    "@[${TMPDIR_TEST}/my folder/test file.txt]"

assert_eq "single-quoted path with spaces in sentence" \
    "$(format "check '${TMPDIR_TEST}/my folder/test file.txt' please")" \
    "check @[${TMPDIR_TEST}/my folder/test file.txt] please"

# Simple path (no spaces)
assert_eq "simple path no spaces" \
    "$(format "${TMPDIR_TEST}/simple.txt")" \
    "@[${TMPDIR_TEST}/simple.txt]"

# Multiple paths
assert_eq "multiple existing paths" \
    "$(format "compare /usr/bin/env and ${TMPDIR_TEST}/simple.txt")" \
    "compare @[/usr/bin/env] and @[${TMPDIR_TEST}/simple.txt]"

assert_eq "mixed existing and nonexistent" \
    "$(format "check /usr/bin/env and /nonexistent/foo.rs")" \
    "check @[/usr/bin/env] and /nonexistent/foo.rs"

# Empty input
assert_eq "empty input" \
    "$(format "")" \
    ""

# Backslash-escaped paths (terminals like Ghostty send /path/my\ file.txt)
local escaped_path="${TMPDIR_TEST}/my\ folder/test\ file.txt"
assert_eq "backslash-escaped path (whole paste)" \
    "$(format "$escaped_path")" \
    "@[${TMPDIR_TEST}/my folder/test file.txt]"

assert_eq "backslash-escaped path in sentence" \
    "$(format "check $escaped_path please")" \
    "check @[${TMPDIR_TEST}/my folder/test file.txt] please"

local escaped_simple="${TMPDIR_TEST}/simple.txt"
assert_eq "path without spaces (no escaping needed)" \
    "$(format "$escaped_simple")" \
    "@[${TMPDIR_TEST}/simple.txt]"

assert_eq "backslash-escaped nonexistent path untouched" \
    "$(format "/nonexistent/my\ folder/file.txt")" \
    "/nonexistent/my\ folder/file.txt"

# --- Performance tests -------------------------------------------------------

echo ""
echo -e "${BOLD}Performance Tests${RESET} ${DIM}— forge zsh format${RESET}"
echo ""

ITERATIONS=20

# Benchmark: simple path
START=$EPOCHREALTIME
for i in $(seq 1 $ITERATIONS); do
    format "look at /usr/bin/env please" > /dev/null
done
END=$EPOCHREALTIME
ELAPSED=$(( (END - START) * 1000 ))
AVG=$(( ELAPSED / ITERATIONS ))
printf "  ${DIM}simple path        ${RESET} ${CYAN}%.2f${RESET} ${DIM}ms avg (${ITERATIONS} iterations)${RESET}\n" $AVG

# Benchmark: path with spaces
START=$EPOCHREALTIME
for i in $(seq 1 $ITERATIONS); do
    format "check '${TMPDIR_TEST}/my folder/test file.txt' please" > /dev/null
done
END=$EPOCHREALTIME
ELAPSED=$(( (END - START) * 1000 ))
AVG=$(( ELAPSED / ITERATIONS ))
printf "  ${DIM}quoted path spaces ${RESET} ${CYAN}%.2f${RESET} ${DIM}ms avg (${ITERATIONS} iterations)${RESET}\n" $AVG

# Benchmark: plain text (no paths)
START=$EPOCHREALTIME
for i in $(seq 1 $ITERATIONS); do
    format "explain how this works in detail" > /dev/null
done
END=$EPOCHREALTIME
ELAPSED=$(( (END - START) * 1000 ))
AVG=$(( ELAPSED / ITERATIONS ))
printf "  ${DIM}plain text         ${RESET} ${CYAN}%.2f${RESET} ${DIM}ms avg (${ITERATIONS} iterations)${RESET}\n" $AVG

# Benchmark: already wrapped
START=$EPOCHREALTIME
for i in $(seq 1 $ITERATIONS); do
    format "check @[/usr/bin/env] and explain" > /dev/null
done
END=$EPOCHREALTIME
ELAPSED=$(( (END - START) * 1000 ))
AVG=$(( ELAPSED / ITERATIONS ))
printf "  ${DIM}already wrapped    ${RESET} ${CYAN}%.2f${RESET} ${DIM}ms avg (${ITERATIONS} iterations)${RESET}\n" $AVG

# --- Cleanup -----------------------------------------------------------------

rm -rf "$TMPDIR_TEST"

# --- Summary -----------------------------------------------------------------

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
