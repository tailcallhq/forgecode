-- Reverse of 2026-06-14-000003_add_parent_id_source_indexes/up.sql.
--
-- The pre-migration state was: a single-column index on `source` only.
-- Recreate it so that a downgrade returns to the prior shape, then drop
-- the composite (workspace_id, parent_id) index.

CREATE INDEX IF NOT EXISTS idx_conversations_source
    ON conversations(source);

DROP INDEX IF EXISTS idx_conversations_workspace_source;
DROP INDEX IF EXISTS idx_conversations_workspace_parent;
