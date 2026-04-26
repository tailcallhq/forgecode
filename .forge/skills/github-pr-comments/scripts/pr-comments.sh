#!/bin/bash

# Extract active (non-resolved, non-outdated) review comment threads from a PR,
# each paired with its surrounding code context (diff hunk).
#
# Usage:
#   ./scripts/pr-comments.sh [PR_NUMBER]
#
# If PR_NUMBER is omitted, the script resolves the PR for the current branch.

set -euo pipefail

# ---------------------------------------------------------------------------
# Resolve PR number
# ---------------------------------------------------------------------------
if [[ $# -ge 1 ]]; then
  PR_NUMBER="$1"
else
  PR_NUMBER=$(gh pr view --json number -q '.number' 2>/dev/null) || {
    echo "error: not on a branch with an open PR — provide a PR number as the first argument." >&2
    exit 1
  }
fi

# ---------------------------------------------------------------------------
# Resolve repository owner and name for GraphQL variables.
# ---------------------------------------------------------------------------
OWNER=$(gh repo view --json owner -q '.owner.login')
REPO=$(gh repo view --json name -q '.name')

# ---------------------------------------------------------------------------
# Fetch review threads via GraphQL.
# The REST comments endpoint does not expose resolution or outdated status.
# GraphQL provides isResolved and isOutdated per thread, which lets us skip
# threads that no longer need action.
# --paginate follows endCursor automatically; jq -s merges all pages.
# gh colorizes output even when piped, so strip ANSI escape codes first.
# ---------------------------------------------------------------------------
STRIP_ANSI=$'s/\033\\[[0-9;]*[mGKH]//g'

THREADS_JSON=$(gh api graphql \
  -f query='
    query($owner: String!, $repo: String!, $pr: Int!, $endCursor: String) {
      repository(owner: $owner, name: $repo) {
        pullRequest(number: $pr) {
          reviewThreads(first: 100, after: $endCursor) {
            pageInfo { hasNextPage endCursor }
            nodes {
              isResolved
              isOutdated
              comments(first: 50) {
                nodes {
                  path
                  line
                  body
                  author { login }
                  createdAt
                  diffHunk
                }
              }
            }
          }
        }
      }
    }
  ' \
  -f owner="$OWNER" \
  -f repo="$REPO" \
  -F pr="$PR_NUMBER" \
  --paginate \
  | sed "$STRIP_ANSI" \
  | jq -s '[.[].data.repository.pullRequest.reviewThreads.nodes[] | select(.isResolved == false and .isOutdated == false)]')

TOTAL=$(echo "$THREADS_JSON" | jq 'length')

if [[ "$TOTAL" -eq 0 ]]; then
  echo "No active review comments found for PR #${PR_NUMBER}."
  exit 0
fi

echo "PR #${PR_NUMBER} — ${TOTAL} active review thread(s)"
echo ""

# ---------------------------------------------------------------------------
# Format and print each active thread in a single jq pass.
# The first comment in the thread carries the diff hunk (code context).
# All comments in the thread are shown so the full conversation is visible.
# ---------------------------------------------------------------------------
SEP="$(printf '%0.s─' {1..80})"

echo "$THREADS_JSON" | jq -r --arg sep "$SEP" '
  .[] |
  (.comments.nodes[0]) as $first |
  [
    $sep,
    ("File   : " + $first.path + ":" + (($first.line // "?") | tostring)),
    ("Author : @" + $first.author.login),
    ("Date   : " + $first.createdAt),
    "",
    "-- code context --",
    $first.diffHunk,
    "",
    "-- comment --",
    (.comments.nodes | map(.body) | join("\n\n---\n\n")),
    ""
  ] | join("\n")
'
