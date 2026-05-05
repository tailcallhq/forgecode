#!/usr/bin/env bash
# Test that a 400 error from an invalid model shows the full response body.
#
# Usage:
#   ./scripts/test-400-error-message.sh [provider_id] [model_id]
#
# Defaults to github_copilot / gpt-5.5 which routes through the Responses API
# path (model contains "gpt-5") and triggers a 400 for an unsupported model.

set -euo pipefail

PROVIDER="${1:-github_copilot}"
MODEL="${2:-gpt-5.5}"
BINARY="target/debug/forge"

echo "Building debug binary..."
cargo build -p forge_main 2>&1 | tail -3

echo ""
echo "Running: FORGE_SESSION__PROVIDER_ID=$PROVIDER FORGE_SESSION__MODEL_ID=$MODEL $BINARY -p 'Hi'"
echo "---"

# Capture stderr (where forge writes errors) and stdout separately.
ERROR_OUTPUT=$(
  FORGE_SESSION__PROVIDER_ID="$PROVIDER" \
  FORGE_SESSION__MODEL_ID="$MODEL" \
  "$BINARY" -p "Hi" 2>&1 || true
)

echo "$ERROR_OUTPUT"
echo "---"

# Check that the error output contains something more than just a status code.
if echo "$ERROR_OUTPUT" | grep -qiE "Reason:|message|error|body"; then
  echo "PASS: Error output contains a descriptive reason (body or message)."
else
  echo "FAIL: Error output does not contain a reason. Only status code shown."
  exit 1
fi
