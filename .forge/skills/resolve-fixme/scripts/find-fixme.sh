#!/usr/bin/env bash
#
# find-fixme.sh — locate all FIXME comments in source files and print each
# occurrence with surrounding context (2 lines before, 5 lines after).
#
# Usage:
#   ./scripts/find-fixme.sh [PATH]
#
# If PATH is omitted the current working directory is searched.
# Skips .git/, target/, node_modules/, and vendor/ directories.

set -euo pipefail

SEARCH_ROOT="${1:-.}"

# ---------------------------------------------------------------------------
# Colours
# ---------------------------------------------------------------------------
BOLD='\033[1m'
RESET='\033[0m'
CYAN='\033[36m'
YELLOW='\033[33m'
DIM='\033[2m'
SEP="$(printf '%0.s─' {1..80})"

CONTEXT_BEFORE=2
CONTEXT_AFTER=5

# ---------------------------------------------------------------------------
# Collect matches into a temp file as "filepath<TAB>linenum" lines.
# Using rg --json + python for robust parsing that handles colons in paths
# and content. Falls back to grep + python when rg is unavailable.
# ---------------------------------------------------------------------------
TMPFILE=$(mktemp)
trap 'rm -f "$TMPFILE"' EXIT

_EXCLUDES='!.git !target !node_modules !vendor'

if command -v rg &>/dev/null; then
    rg --json --case-sensitive \
        --glob '!.git' --glob '!target' --glob '!node_modules' --glob '!vendor' \
        'FIXME' "$SEARCH_ROOT" 2>/dev/null \
    | python3 -c "
import sys, json
for line in sys.stdin:
    try:
        obj = json.loads(line)
        if obj.get('type') == 'match':
            data = obj['data']
            path = data['path']['text']
            linenum = data['line_number']
            print(f'{path}\t{linenum}')
    except Exception:
        pass
" > "$TMPFILE" || true
else
    grep -rn \
        --exclude-dir='.git' --exclude-dir='target' \
        --exclude-dir='node_modules' --exclude-dir='vendor' \
        'FIXME' "$SEARCH_ROOT" 2>/dev/null \
    | python3 -c "
import sys, re
for line in sys.stdin:
    # grep -n format: filepath:linenum:content
    # The linenum is always digits, so match greedily from the right
    m = re.match(r'^(.*):([0-9]+):', line)
    if m:
        print(m.group(1) + '\t' + m.group(2))
" > "$TMPFILE" || true
fi

TOTAL=$(wc -l < "$TMPFILE" | tr -d ' ')

if [[ "$TOTAL" -eq 0 ]]; then
    echo "No FIXME comments found in: $SEARCH_ROOT"
    exit 0
fi

echo -e "${BOLD}Found ${YELLOW}${TOTAL}${RESET}${BOLD} FIXME comment(s) in: ${SEARCH_ROOT}${RESET}"
echo ""

COUNT=0

while IFS=$'\t' read -r FIXME_FILE FIXME_LINE; do
    [[ -z "$FIXME_FILE" || -z "$FIXME_LINE" ]] && continue
    [[ ! "$FIXME_LINE" =~ ^[0-9]+$ ]]          && continue
    [[ ! -f "$FIXME_FILE" ]]                    && continue

    COUNT=$((COUNT + 1))

    START=$(( FIXME_LINE - CONTEXT_BEFORE ))
    [[ $START -lt 1 ]] && START=1
    END=$(( FIXME_LINE + CONTEXT_AFTER ))

    echo -e "${SEP}"
    echo -e "${BOLD}${CYAN}[${COUNT}/${TOTAL}] ${FIXME_FILE}:${FIXME_LINE}${RESET}"
    echo ""

    # Print lines with line numbers, highlighting the FIXME line
    LINE_IDX=$START
    while IFS= read -r file_line; do
        if [[ "$LINE_IDX" -eq "$FIXME_LINE" ]]; then
            echo -e "  ${YELLOW}${LINE_IDX}:${RESET}  ${YELLOW}${file_line}${RESET}"
        else
            echo -e "  ${DIM}${LINE_IDX}:${RESET}  ${file_line}"
        fi
        LINE_IDX=$((LINE_IDX + 1))
    done < <(sed -n "${START},${END}p" "$FIXME_FILE")

    echo ""
done < "$TMPFILE"

echo -e "${SEP}"
echo -e "${BOLD}Total: ${YELLOW}${COUNT}${RESET}${BOLD} FIXME(s)${RESET}"
