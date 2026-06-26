-- Rollback zstd compression (best-effort)
-- Note: SQLite < 3.35 cannot easily DROP COLUMN; this is for future compatibility
-- If rollback is needed before column-drop support, manually delete context_zstd data
-- and set is_compressed=0 for all rows, then restore to plain context column.

DROP INDEX IF EXISTS idx_conversations_compressed;

-- SQLite 3.35+ only: uncomment to enable
-- ALTER TABLE conversations DROP COLUMN context_zstd;
-- ALTER TABLE conversations DROP COLUMN is_compressed;

-- Forward-only policy: no destructive rollback
-- To rollback, restore from backup or manually migrate compressed rows back to context column
