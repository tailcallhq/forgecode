#!/usr/bin/env bash
#
# Integration tests for the /undo feature.
#
# Creates a test file, modifies it 3 times in 3 prompts, then sequentially
# undoes each change — verifying file content and snapshot_metadata DB state
# at every step.
#
# Usage:
#   ./scripts/integration/test_undo.sh
#
# Prerequisites:
#   - A valid forge provider configured (uses your existing ~/forge config)
#   - sqlite3 on PATH
#
set -uo pipefail

FORGE_BIN="./target/debug/forge"
DB_PATH="$HOME/forge/.forge.db"
TEST_DIR="/tmp/forge_itest_undo_$$"
TEST_FILE="$TEST_DIR/test_file.txt"

# ── Colors ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

PASS=0
FAIL=0
CONVERSATION_ID=""

# ── Helpers ─────────────────────────────────────────────────────────────────

assert_eq() {
    local description="$1" actual="$2" expected="$3"
    if [[ "$actual" == "$expected" ]]; then
        echo -e "  ${GREEN}PASS${NC}: $description"
        ((PASS++))
    else
        echo -e "  ${RED}FAIL${NC}: $description"
        echo -e "    expected: $expected"
        echo -e "    actual:   $actual"
        ((FAIL++))
    fi
}

assert_contains() {
    local description="$1" haystack="$2" needle="$3"
    if [[ "$haystack" == *"$needle"* ]]; then
        echo -e "  ${GREEN}PASS${NC}: $description"
        ((PASS++))
    else
        echo -e "  ${RED}FAIL${NC}: $description"
        echo -e "    expected to contain: $needle"
        echo -e "    actual: $haystack"
        ((FAIL++))
    fi
}

assert_file_content() {
    local description="$1" file_path="$2" expected="$3"
    if [[ -f "$file_path" ]]; then
        local actual
        actual=$(cat "$file_path")
        assert_eq "$description" "$actual" "$expected"
    else
        echo -e "  ${RED}FAIL${NC}: $description — file does not exist: $file_path"
        ((FAIL++))
    fi
}

assert_file_not_exists() {
    local description="$1" file_path="$2"
    if [[ ! -f "$file_path" ]]; then
        echo -e "  ${GREEN}PASS${NC}: $description"
        ((PASS++))
    else
        echo -e "  ${RED}FAIL${NC}: $description — file still exists: $file_path"
        ((FAIL++))
    fi
}

assert_true() {
    local description="$1" actual="$2"
    if [[ "$actual" == "true" ]]; then
        echo -e "  ${GREEN}PASS${NC}: $description"
        ((PASS++))
    else
        echo -e "  ${RED}FAIL${NC}: $description"
        echo -e "    expected: true"
        echo -e "    actual:   $actual"
        ((FAIL++))
    fi
}

# Run forge -p and return the output. Reuses CONVERSATION_ID if set.
forge_prompt() {
    local prompt="$1"
    local conv_flag=""
    if [[ -n "$CONVERSATION_ID" ]]; then
        conv_flag="--conversation-id $CONVERSATION_ID"
    fi
    "$FORGE_BIN" -C "$TEST_DIR" -p "$prompt" $conv_flag 2>&1 || true
}

# Run forge --undo and return the output. Requires CONVERSATION_ID to be set.
forge_undo() {
    "$FORGE_BIN" -C "$TEST_DIR" --undo --conversation-id "$CONVERSATION_ID" 2>&1 || true
}

# Extract conversation ID from forge output (appears on "Initialize" or "Continue" lines).
extract_conversation_id() {
    local output="$1"
    # --color=never is critical: grep adds ANSI color codes to matches by default
    CONVERSATION_ID=$(echo "$output" | grep --color=never -oE '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}' | head -1)
}
# DB helpers — query snapshot_metadata for the current conversation.
db_count_rows() {
    local filter="${1:-}"
    local query="SELECT COUNT(*) FROM snapshot_metadata WHERE conversation_id = '$CONVERSATION_ID'"
    [[ -n "$filter" ]] && query="$query AND $filter"
    local result
    result=$(sqlite3 "$DB_PATH" "$query" 2>&1) || result="0"
    echo "$result"
}

db_count_active()  { db_count_rows "undone_at IS NULL"; }
db_count_undone()  { db_count_rows "undone_at IS NOT NULL"; }

db_count_distinct_user_input_ids() {
    sqlite3 "$DB_PATH" "SELECT COUNT(DISTINCT user_input_id) FROM snapshot_metadata WHERE conversation_id = '$CONVERSATION_ID'" 2>/dev/null || echo "0"
}

db_get_snap_file_path() {
    local file_path="$1"
    # Use LIKE to handle macOS /tmp -> /private/tmp symlink resolution
    local basename
    basename=$(basename "$file_path")
    sqlite3 "$DB_PATH" "SELECT snap_file_path FROM snapshot_metadata WHERE conversation_id = '$CONVERSATION_ID' AND file_path LIKE '%$basename' AND undone_at IS NULL ORDER BY created_at DESC LIMIT 1" 2>/dev/null || echo ""
}

db_is_undone() {
    local file_path="$1"
    local basename
    basename=$(basename "$file_path")
    local result
    result=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM snapshot_metadata WHERE conversation_id = '$CONVERSATION_ID' AND file_path LIKE '%$basename' AND undone_at IS NOT NULL" 2>/dev/null || echo "0")
    if [[ "$result" -gt 0 ]]; then
        echo "true"
    else
        echo "false"
    fi
}

# ── Setup ───────────────────────────────────────────────────────────────────

echo ""
echo "========================================"
echo "  Undo Integration Tests"
echo "========================================"
echo ""

# Build
echo "Building forge..."
cargo build 2>&1 | tail -1

# Clean up and create test directory
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"

# Create initial test file
echo "original content" > "$TEST_FILE"

echo "Test directory: $TEST_DIR"
echo "Test file:      $TEST_FILE"
echo "DB path:        $DB_PATH"
echo ""

# ── Test 1: Three sequential modifications ──────────────────────────────────

echo "--- Test 1: Three sequential modifications ---"

# Prompt 1
echo -e "${YELLOW}Prompt 1: overwrite with 'change1'${NC}"
OUTPUT1=$(forge_prompt "overwrite $TEST_FILE with change1")
extract_conversation_id "$OUTPUT1"
assert_contains "Prompt 1 executed" "$OUTPUT1" "change1"
assert_file_content "File content after prompt 1" "$TEST_FILE" "change1"
assert_eq "DB rows after prompt 1" "$(db_count_rows)" "1"
assert_eq "DB active rows after prompt 1" "$(db_count_active)" "1"
assert_eq "DB undone rows after prompt 1" "$(db_count_undone)" "0"

# Prompt 2
echo -e "${YELLOW}Prompt 2: overwrite with 'change2'${NC}"
OUTPUT2=$(forge_prompt "overwrite $TEST_FILE with change2")
assert_contains "Prompt 2 executed" "$OUTPUT2" "change2"
assert_file_content "File content after prompt 2" "$TEST_FILE" "change2"
assert_eq "DB rows after prompt 2" "$(db_count_rows)" "2"
assert_eq "DB active rows after prompt 2" "$(db_count_active)" "2"
assert_eq "DB undone rows after prompt 2" "$(db_count_undone)" "0"

# Prompt 3
echo -e "${YELLOW}Prompt 3: overwrite with 'change3'${NC}"
OUTPUT3=$(forge_prompt "overwrite $TEST_FILE with change3")
assert_contains "Prompt 3 executed" "$OUTPUT3" "change3"
assert_file_content "File content after prompt 3" "$TEST_FILE" "change3"
assert_eq "DB rows after prompt 3" "$(db_count_rows)" "3"
assert_eq "DB active rows after prompt 3" "$(db_count_active)" "3"
assert_eq "DB undone rows after prompt 3" "$(db_count_undone)" "0"
assert_eq "3 distinct user_input_ids" "$(db_count_distinct_user_input_ids)" "3"

echo ""

# ── Test 2: Sequential undo — undo all 3 prompts ───────────────────────────

echo "--- Test 2: Sequential undo (3 undos) ---"

# Undo 1: undo prompt 3 → file reverts to "change2"
echo -e "${YELLOW}Undo 1: undo last prompt${NC}"
UNDO1=$(forge_undo)
assert_contains "Undo 1 executed" "$UNDO1" "Restored"
assert_file_content "File content after undo 1" "$TEST_FILE" "change2"
assert_eq "DB active rows after undo 1" "$(db_count_active)" "2"
assert_eq "DB undone rows after undo 1" "$(db_count_undone)" "1"

# Undo 2: undo prompt 2 → file reverts to "change1"
echo -e "${YELLOW}Undo 2: undo last prompt${NC}"
UNDO2=$(forge_undo)
assert_contains "Undo 2 executed" "$UNDO2" "Restored"
assert_file_content "File content after undo 2" "$TEST_FILE" "change1"
assert_eq "DB active rows after undo 2" "$(db_count_active)" "1"
assert_eq "DB undone rows after undo 2" "$(db_count_undone)" "2"

# Undo 3: undo prompt 1 → file reverts to "original content"
echo -e "${YELLOW}Undo 3: undo last prompt${NC}"
UNDO3=$(forge_undo)
assert_contains "Undo 3 executed" "$UNDO3" "Restored"
assert_file_content "File content after undo 3" "$TEST_FILE" "original content"
assert_eq "DB active rows after undo 3" "$(db_count_active)" "0"
assert_eq "DB undone rows after undo 3" "$(db_count_undone)" "3"

echo ""

# ── Test 3: Undo with no active snapshots ───────────────────────────────────

echo "--- Test 3: Undo with no active snapshots ---"
UNDO4=$(forge_undo)
assert_contains "Undo 4 reports no changes" "$UNDO4" "No file changes"

echo ""

# ── Test 4: New file creation and undo (file should be deleted) ─────────────

echo "--- Test 4: New file creation and undo ---"

NEW_FILE="$TEST_DIR/brand_new.txt"

echo -e "${YELLOW}Create new file${NC}"
CREATE_OUTPUT=$(forge_prompt "create $NEW_FILE with brand new content")
assert_contains "New file created" "$CREATE_OUTPUT" "brand new"
assert_file_content "New file has correct content" "$NEW_FILE" "brand new content"

# Verify DB row has empty snap_file_path for new files
SNAP_PATH=$(db_get_snap_file_path "$NEW_FILE")
assert_eq "snap_file_path is empty for new file" "$SNAP_PATH" ""

echo -e "${YELLOW}Undo new file creation${NC}"
UNDO_NEW=$(forge_undo)
assert_contains "Undo new file executed" "$UNDO_NEW" "Deleted"
assert_file_not_exists "New file deleted after undo" "$NEW_FILE"
assert_true "New file row marked undone in DB" "$(db_is_undone "$NEW_FILE")"

echo ""

# ── Test 5: Mixed modify + create in one prompt, then undo ──────────────────

echo "--- Test 5: Mixed modify + create, then undo ---"

# Reset test file
echo "original content" > "$TEST_FILE"
MIXED_FILE="$TEST_DIR/mixed_new.txt"

echo -e "${YELLOW}Modify existing + create new in one prompt${NC}"
MIXED_OUTPUT=$(forge_prompt "overwrite $TEST_FILE with mixed change AND create $MIXED_FILE with mixed new file")
assert_contains "Mixed prompt executed" "$MIXED_OUTPUT" "mixed"
assert_file_content "Existing file modified" "$TEST_FILE" "mixed change"
assert_file_content "New file created" "$MIXED_FILE" "mixed new file"

# Capture active rows AFTER the mixed prompt (before undo)
ACTIVE_BEFORE=$(db_count_active)

echo -e "${YELLOW}Undo mixed prompt${NC}"
MIXED_UNDO=$(forge_undo)
assert_contains "Mixed undo executed" "$MIXED_UNDO" "Restored"
assert_file_content "Existing file restored" "$TEST_FILE" "original content"
assert_file_not_exists "New file deleted" "$MIXED_FILE"

ACTIVE_AFTER=$(db_count_active)
assert_eq "Active rows decreased by 2 after mixed undo" "$((ACTIVE_BEFORE - ACTIVE_AFTER))" "2"

echo ""

# ── Test 6: Manually deleted new file — undo should tolerate ────────────────

echo "--- Test 6: Manually deleted new file — undo tolerates ---"

MANUAL_FILE="$TEST_DIR/manual_delete.txt"

echo -e "${YELLOW}Create file for manual delete test${NC}"
MANUAL_CREATE=$(forge_prompt "create $MANUAL_FILE with will be deleted manually")
assert_contains "File created" "$MANUAL_CREATE" "deleted manually"

echo -e "${YELLOW}Manually delete the file${NC}"
rm -f "$MANUAL_FILE"
assert_file_not_exists "File manually deleted" "$MANUAL_FILE"

echo -e "${YELLOW}Undo — should succeed even though file is gone${NC}"
MANUAL_UNDO=$(forge_undo)
assert_contains "Undo of manually deleted file succeeded" "$MANUAL_UNDO" "Deleted"

echo ""

# ── Test 7: Undo deletes multiple newly created files from last prompt ──────

echo "--- Test 7: Undo deletes multiple newly created files ---"

NEW_FILE_A="$TEST_DIR/new_a.txt"
NEW_FILE_B="$TEST_DIR/new_b.txt"
NEW_FILE_C="$TEST_DIR/new_c.txt"

echo -e "${YELLOW}Create 3 new files in one prompt${NC}"
MULTI_CREATE=$(forge_prompt "create $NEW_FILE_A with content A AND create $NEW_FILE_B with content B AND create $NEW_FILE_C with content C")
assert_contains "Multi-create executed" "$MULTI_CREATE" "content"
assert_file_content "New file A has correct content" "$NEW_FILE_A" "content A"
assert_file_content "New file B has correct content" "$NEW_FILE_B" "content B"
assert_file_content "New file C has correct content" "$NEW_FILE_C" "content C"

ACTIVE_BEFORE_MULTI_CREATE=$(db_count_active)

echo -e "${YELLOW}Undo — all 3 new files should be deleted${NC}"
MULTI_CREATE_UNDO=$(forge_undo)
assert_contains "Multi-create undo executed" "$MULTI_CREATE_UNDO" "Deleted"
assert_file_not_exists "New file A deleted after undo" "$NEW_FILE_A"
assert_file_not_exists "New file B deleted after undo" "$NEW_FILE_B"
assert_file_not_exists "New file C deleted after undo" "$NEW_FILE_C"
assert_true "New file A row marked undone in DB" "$(db_is_undone "$NEW_FILE_A")"
assert_true "New file B row marked undone in DB" "$(db_is_undone "$NEW_FILE_B")"
assert_true "New file C row marked undone in DB" "$(db_is_undone "$NEW_FILE_C")"

ACTIVE_AFTER_MULTI_CREATE=$(db_count_active)
assert_eq "Active rows decreased by 3 after multi-create undo" "$((ACTIVE_BEFORE_MULTI_CREATE - ACTIVE_AFTER_MULTI_CREATE))" "3"

echo ""

# ── Test 8: Multi-file edits — one prompt edits multiple existing files ──────

echo "--- Test 8: Multi-file edits — undo reverses all ---"

MULTI_EDIT_A="$TEST_DIR/edit_a.txt"
MULTI_EDIT_B="$TEST_DIR/edit_b.txt"
MULTI_EDIT_C="$TEST_DIR/edit_c.txt"

# Create 3 files with original content
echo "original A" > "$MULTI_EDIT_A"
echo "original B" > "$MULTI_EDIT_B"
echo "original C" > "$MULTI_EDIT_C"

echo -e "${YELLOW}Edit all 3 files in one prompt${NC}"
MULTI_EDIT=$(forge_prompt "overwrite $MULTI_EDIT_A with modified A AND overwrite $MULTI_EDIT_B with modified B AND overwrite $MULTI_EDIT_C with modified C")
assert_contains "Multi-edit executed" "$MULTI_EDIT" "modified"
assert_file_content "File A modified" "$MULTI_EDIT_A" "modified A"
assert_file_content "File B modified" "$MULTI_EDIT_B" "modified B"
assert_file_content "File C modified" "$MULTI_EDIT_C" "modified C"

ACTIVE_BEFORE_MULTI_EDIT=$(db_count_active)

echo -e "${YELLOW}Undo — all 3 files should revert to original content${NC}"
MULTI_EDIT_UNDO=$(forge_undo)
assert_contains "Multi-edit undo executed" "$MULTI_EDIT_UNDO" "Restored"
assert_file_content "File A reverted" "$MULTI_EDIT_A" "original A"
assert_file_content "File B reverted" "$MULTI_EDIT_B" "original B"
assert_file_content "File C reverted" "$MULTI_EDIT_C" "original C"

ACTIVE_AFTER_MULTI_EDIT=$(db_count_active)
assert_eq "Active rows decreased by 3 after multi-edit undo" "$((ACTIVE_BEFORE_MULTI_EDIT - ACTIVE_AFTER_MULTI_EDIT))" "3"

echo ""

# ── Cleanup & Results ───────────────────────────────────────────────────────

echo ""
echo "========================================"
echo "  Results"
echo "========================================"
echo -e "  ${GREEN}Passed: $PASS${NC}"
echo -e "  ${RED}Failed: $FAIL${NC}"
echo ""

rm -rf "$TEST_DIR"

if [[ "$FAIL" -gt 0 ]]; then
    exit 1
fi
exit 0
