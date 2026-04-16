#!/bin/bash

# Skill Search Quality Eval Runner
#
# This script runs the skill search quality evaluation without any LLM judge.
# All validation is deterministic using shell and jq.
#
# Usage: ./run_eval.sh <context_file>

set -e

CONTEXT_FILE="$1"
if [ -z "$CONTEXT_FILE" ]; then
  echo "Usage: $0 <context_file>"
  exit 1
fi

if [ ! -f "$CONTEXT_FILE" ]; then
  echo "✗ Context file not found: $CONTEXT_FILE"
  exit 1
fi

echo "=========================================="
echo "Running Skill Search Quality Eval"
echo "=========================================="
echo ""

# Step 1: Check if skill_search was used
echo "Step 1: Checking for skill_search tool usage..."
if cat "$CONTEXT_FILE" | jq -e '[.messages[]?.tool_calls[]? | select(.function.name == "skill_search")] | any' > /dev/null 2>&1; then
  echo "✓ skill_search tool found in context"
else
  echo "✗ FAILED: skill_search tool not used"
  exit 1
fi

# Step 2: Extract skill_search results
echo ""
echo "Step 2: Extracting skill_search results..."

# Try to extract skill names from the tool response
SKILL_RESULTS=$(cat "$CONTEXT_FILE" | jq -r '
  .messages[]? | 
  select(.role == "tool" and .name == "skill_search") | 
  .content' 2>/dev/null)

if [ -z "$SKILL_RESULTS" ]; then
  echo "✗ No skill_search results found in context"
  exit 1
fi

# Try to parse as XML first, then fallback to line-by-line
SKILL_NAMES=$(echo "$SKILL_RESULTS" | tr '<' '\n' | grep '^name>' | cut -d'>' -f2 | cut -d'<' -f1 2>/dev/null || echo "")

if [ -z "$SKILL_NAMES" ]; then
  # Fallback: try to extract skill names from plain text format
  # Skills are typically listed as "name: description" or just "name"
  SKILL_NAMES=$(echo "$SKILL_RESULTS" | grep -E '^[a-z0-9-]+' | head -10)
fi

if [ -z "$SKILL_NAMES" ]; then
  echo "⚠ Could not extract skill names from results"
  echo "Raw results:"
  echo "$SKILL_RESULTS" | head -20
else
  echo "Skills found in results:"
  echo "$SKILL_NAMES" | sed 's/^/  - /'
fi

# Step 3: Extract the query used
echo ""
echo "Step 3: Extracting skill_search query..."
QUERY=$(cat "$CONTEXT_FILE" | jq -r '
  [.messages[]?.tool_calls[]? | select(.function.name == "skill_search")][0] | 
  .function.arguments' 2>/dev/null | jq -r '.query // "(not found)"')

echo "Query: $QUERY"

# Step 4: Count skills returned
echo ""
echo "Step 4: Counting skills returned..."
SKILL_COUNT=$(echo "$SKILL_NAMES" | grep -v '^$' | wc -l | tr -d ' ')
echo "Number of skills returned: $SKILL_COUNT"

# Step 5: Validate expected skills (if provided)
if [ -n "$EXPECTED_SKILLS" ]; then
  echo ""
  echo "Step 5: Validating expected skills..."
  echo "Expected: $EXPECTED_SKILLS"
  
  IFS=',' read -ra EXPECTED <<< "$EXPECTED_SKILLS"
  MISSING=()
  
  for skill in "${EXPECTED[@]}"; do
    skill=$(echo "$skill" | xargs) # trim
    if ! echo "$SKILL_NAMES" | grep -qF "$skill"; then
      MISSING+=("$skill")
    fi
  done
  
  if [ ${#MISSING[@]} -eq 0 ]; then
    echo "✓ All expected skills found"
  else
    echo "✗ Missing expected skills:"
    printf '  - %s\n' "${MISSING[@]}"
    exit 1
  fi
fi

# Step 6: Validate must_be_first (if provided)
if [ -n "$MUST_BE_FIRST" ]; then
  echo ""
  echo "Step 6: Validating must_be_first requirement..."
  echo "Expected first: $MUST_BE_FIRST"
  
  FIRST_SKILL=$(echo "$SKILL_NAMES" | head -1)
  echo "Actual first: $FIRST_SKILL"
  
  if [ "$FIRST_SKILL" = "$MUST_BE_FIRST" ]; then
    echo "✓ First skill matches requirement"
  else
    echo "✗ First skill mismatch"
    exit 1
  fi
fi

# Final summary
echo ""
echo "=========================================="
echo "✓ EVALUATION PASSED"
echo "=========================================="
echo ""
echo "Summary:"
echo "  - skill_search tool was called"
echo "  - Query: ${QUERY}"
echo "  - Skills returned: ${SKILL_COUNT}"
if [ -n "$EXPECTED_SKILLS" ]; then
  echo "  - Expected skills: ✓"
fi
if [ -n "$MUST_BE_FIRST" ]; then
  echo "  - Must-be-first: ✓"
fi

exit 0
