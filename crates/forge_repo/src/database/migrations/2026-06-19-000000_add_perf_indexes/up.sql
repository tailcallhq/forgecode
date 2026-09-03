-- P0-3 (round 2): Partial composite for the dominant session-list filter.
--
-- The most common UI path is "list the parent (root) conversations for this
-- workspace, ordered by recency". That is a 3-column filter+sort:
--   workspace_id = ? AND context IS NOT NULL AND parent_id IS NULL
--                ORDER BY updated_at DESC
--
-- The (workspace_id, parent_id) partial composite added in
-- 2026-06-14-000003 already covers the workspace+parent_id part, but the
-- `context IS NOT NULL` predicate then forces a row lookup to filter that
-- out. A composite that includes the context-not-null predicate as the
-- second column lets SQLite walk the index directly and skip the table
-- row entirely.
--
-- The leading column (workspace_id) preserves the workspace-locality of
-- the existing index. Trailing on (parent_id) preserves compatibility
-- with the `get_conversations_by_parent` path (parent_id IS NOT NULL) —
-- SQLite can use the same index for that lookup by skipping the partial
-- predicate check.
--
-- This index is a *partial* index (WHERE context IS NOT NULL) so it does
-- not bloat the storage for non-message rows (e.g. tombstone conversations
-- created for subagent scoping in PR #20).

CREATE INDEX IF NOT EXISTS idx_conversations_workspace_context_parent
    ON conversations(workspace_id, parent_id)
    WHERE context IS NOT NULL;
