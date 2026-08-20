-- P0-3: Composite indexes for the hot session-history queries.
--
-- Without these indexes, every call to:
--   - get_conversations_by_parent  (parent_id = ?)
--   - get_parent_conversations     (parent_id IS NULL)
--   - get_conversations_by_source  (source = ?)
-- triggers a full scan of the workspace partition. For a workspace with
-- thousands of stored sessions this dominates the per-list / per-pick
-- latency.
--
-- The composite (workspace_id, parent_id) and (workspace_id, source)
-- ordering lets SQLite walk the index in workspace order (which is
-- already the dominant filter) and avoid touching rows that belong to
-- a different workspace.
--
-- The `WHERE context IS NOT NULL` partial predicate matches the
-- application filter, so the index only stores rows that the list
-- paths can ever return.

CREATE INDEX IF NOT EXISTS idx_conversations_workspace_parent
    ON conversations(workspace_id, parent_id)
    WHERE context IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_conversations_workspace_source
    ON conversations(workspace_id, source)
    WHERE context IS NOT NULL;

-- An index on source alone (without workspace_id) was created by the
-- prior 2026-06-14-000001 migration. The composite (workspace_id,
-- source) above strictly dominates it for any query that filters on
-- workspace_id, so the single-column index becomes dead weight.
DROP INDEX IF EXISTS idx_conversations_source;
