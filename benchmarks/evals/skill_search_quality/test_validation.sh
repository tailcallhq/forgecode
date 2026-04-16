#!/bin/bash

# Test Validation Script for Skill Search Quality Eval
#
# This script tests the validation logic without requiring actual eval runs.
# It creates mock context files and validates the jq extraction logic.
#
# Usage: ./test_validation.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR="/tmp/test_skill_search_eval_$$"

echo "======================================"
echo "Testing Skill Search Quality Eval"
echo "======================================"
echo

mkdir -p "$TEST_DIR"

# Test 1: skill_search tool detection
echo "Test 1: Check that skill_search tool detection works..."
cat > "$TEST_DIR/context.json" <<'EOF'
{
  "messages": [
    {
      "role": "assistant",
      "tool_calls": [
        {
          "id": "call_1",
          "type": "function",
          "function": {
            "name": "skill_search",
            "arguments": "{\"query\":\"generate PDF document\",\"limit\":5}"
          }
        }
      ]
    }
  ]
}
EOF

if cat "$TEST_DIR/context.json" | jq -e '[.messages[]?.tool_calls[]? | select(.function.name == "skill_search")] | any' > /dev/null 2>&1; then
  echo "✓ Test 1 PASSED: skill_search tool detected correctly"
else
  echo "✗ Test 1 FAILED: skill_search tool not detected"
  exit 1
fi
echo

# Test 2: Missing skill_search detection
echo "Test 2: Check that missing skill_search is detected..."
cat > "$TEST_DIR/context2.json" <<'EOF'
{
  "messages": [
    {
      "role": "assistant",
      "tool_calls": [
        {
          "id": "call_1",
          "type": "function",
          "function": {
            "name": "fs_search",
            "arguments": "{\"pattern\":\"test\"}"
          }
        }
      ]
    }
  ]
}
EOF

if ! cat "$TEST_DIR/context2.json" | jq -e '[.messages[]?.tool_calls[]? | select(.function.name == "skill_search")] | any' > /dev/null 2>&1; then
  echo "✓ Test 2 PASSED: Missing skill_search detected correctly"
else
  echo "✗ Test 2 FAILED: False positive for skill_search"
  exit 1
fi
echo

# Test 3: Extract skill names from XML format
echo "Test 3: Extract skill names from XML-formatted results..."
cat > "$TEST_DIR/context3.json" <<'EOF'
{
  "messages": [
    {
      "role": "tool",
      "name": "skill_search",
      "content": "<results><skill><name>pdf</name><description>Generate PDF documents</description></skill><skill><name>docx</name><description>Generate Word documents</description></skill></results>"
    }
  ]
}
EOF

SKILLS=$(cat "$TEST_DIR/context3.json" | jq -r '
  .messages[]? | 
  select(.role == "tool" and .name == "skill_search") | 
  .content' 2>/dev/null | tr '<' '\n' | grep '^name>' | cut -d'>' -f2 | cut -d'<' -f1 | head -10)

if echo "$SKILLS" | grep -q "pdf" && echo "$SKILLS" | grep -q "docx"; then
  echo "✓ Test 3 PASSED: Skill names extracted from XML"
else
  echo "✗ Test 3 FAILED: Could not extract skill names"
  echo "Got: $SKILLS"
  exit 1
fi
echo

# Test 4: Extract skill names from plain text format
echo "Test 4: Extract skill names from plain text results..."
cat > "$TEST_DIR/context4.json" <<'EOF'
{
  "messages": [
    {
      "role": "tool",
      "name": "skill_search",
      "content": "pdf: Generate PDF documents from content\ndocx: Generate Word documents\nmarkdown: Markdown processing utilities"
    }
  ]
}
EOF

SKILLS=$(cat "$TEST_DIR/context4.json" | jq -r '
  .messages[]? | 
  select(.role == "tool" and .name == "skill_search") | 
  .content' 2>/dev/null | grep -oE '^[a-z0-9-]+' | head -10)

if echo "$SKILLS" | grep -q "pdf" && echo "$SKILLS" | grep -q "docx"; then
  echo "✓ Test 4 PASSED: Skill names extracted from plain text"
else
  echo "✗ Test 4 FAILED: Could not extract skill names from plain text"
  echo "Got: $SKILLS"
  exit 1
fi
echo

# Test 5: Check expected skills validation
echo "Test 5: Validate expected skills matching..."
EXPECTED="pdf,docx,markdown"
SKILLS_FOUND="pdf\ndocx\nmarkdown\nlatex"

IFS=',' read -ra EXPECTED_ARR <<< "$EXPECTED"
MISSING=()

for skill in "${EXPECTED_ARR[@]}"; do
  skill=$(echo "$skill" | xargs)
  if ! echo -e "$SKILLS_FOUND" | grep -qF "$skill"; then
    MISSING+=("$skill")
  fi
done

if [ ${#MISSING[@]} -eq 0 ]; then
  echo "✓ Test 5 PASSED: All expected skills found"
else
  echo "✗ Test 5 FAILED: Missing skills: ${MISSING[*]}"
  exit 1
fi
echo

# Test 6: Check must_be_first validation
echo "Test 6: Validate must_be_first requirement..."
MUST_BE_FIRST="pdf"
FIRST_SKILL=$(echo -e "$SKILLS_FOUND" | head -1)

if [ "$FIRST_SKILL" = "$MUST_BE_FIRST" ]; then
  echo "✓ Test 6 PASSED: First skill matches requirement"
else
  echo "✗ Test 6 FAILED: Expected '$MUST_BE_FIRST', got '$FIRST_SKILL'"
  exit 1
fi
echo

# Test 7: Extract query from tool arguments
echo "Test 7: Extract query from skill_search tool arguments..."
cat > "$TEST_DIR/context7.json" <<'EOF'
{
  "messages": [
    {
      "role": "assistant",
      "tool_calls": [
        {
          "id": "call_1",
          "type": "function",
          "function": {
            "name": "skill_search",
            "arguments": "{\"query\":\"generate PDF from markdown\",\"limit\":5}"
          }
        }
      ]
    }
  ]
}
EOF

QUERY=$(cat "$TEST_DIR/context7.json" | jq -r '
  [.messages[]?.tool_calls[]? | select(.function.name == "skill_search")][0] | 
  .function.arguments' 2>/dev/null | jq -r '.query // "(not found)"')

if [ "$QUERY" = "generate PDF from markdown" ]; then
  echo "✓ Test 7 PASSED: Query extracted correctly"
else
  echo "✗ Test 7 FAILED: Query extraction failed"
  echo "Got: $QUERY"
  exit 1
fi
echo

# Test 8: Empty results handling
echo "Test 8: Handle empty skill_search results gracefully..."
cat > "$TEST_DIR/context8.json" <<'EOF'
{
  "messages": [
    {
      "role": "tool",
      "name": "skill_search",
      "content": ""
    }
  ]
}
EOF

SKILLS=$(cat "$TEST_DIR/context8.json" | jq -r '
  .messages[]? | 
  select(.role == "tool" and .name == "skill_search") | 
  .content' 2>/dev/null | tr '<' '\n' | grep '^name>' | cut -d'>' -f2 | cut -d'<' -f1 | head -10)

if [ -z "$SKILLS" ]; then
  echo "✓ Test 8 PASSED: Empty results handled correctly"
else
  echo "✗ Test 8 FAILED: Should have empty results"
  exit 1
fi
echo

# Test 9: Multiple skill_search calls
echo "Test 9: Handle multiple skill_search calls..."
cat > "$TEST_DIR/context9.json" <<'EOF'
{
  "messages": [
    {
      "role": "assistant",
      "tool_calls": [
        {
          "id": "call_1",
          "function": {
            "name": "skill_search",
            "arguments": "{\"query\":\"first search\"}"
          }
        }
      ]
    },
    {
      "role": "tool",
      "name": "skill_search",
      "content": "pdf: First result"
    },
    {
      "role": "assistant",
      "tool_calls": [
        {
          "id": "call_2",
          "function": {
            "name": "skill_search",
            "arguments": "{\"query\":\"second search\"}"
          }
        }
      ]
    },
    {
      "role": "tool",
      "name": "skill_search",
      "content": "docx: Second result"
    }
  ]
}
EOF

CALL_COUNT=$(cat "$TEST_DIR/context9.json" | jq '[.messages[]?.tool_calls[]? | select(.function.name == "skill_search")] | length')

if [ "$CALL_COUNT" -eq 2 ]; then
  echo "✓ Test 9 PASSED: Multiple skill_search calls detected ($CALL_COUNT calls)"
else
  echo "✗ Test 9 FAILED: Expected 2 calls, got $CALL_COUNT"
  exit 1
fi
echo

# Cleanup
rm -rf "$TEST_DIR"

echo "======================================"
echo "✓ All Tests Passed"
echo "======================================"
echo
echo "The validation logic is working correctly."
echo "You can now run the full eval with confidence."
exit 0
