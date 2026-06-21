-- Reverse of 2026-06-19-000000_add_perf_indexes/up.sql.
--
-- Drops the partial composite (workspace_id, parent_id) WHERE context IS NOT NULL.
-- Downgrade returns to the 2026-06-14-000003 state where the parent-id path is
-- covered by the (workspace_id, parent_id) index without a partial predicate.

DROP INDEX IF EXISTS idx_conversations_workspace_context_parent;
