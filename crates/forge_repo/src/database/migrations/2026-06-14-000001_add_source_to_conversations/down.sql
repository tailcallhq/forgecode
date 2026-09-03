DROP INDEX IF EXISTS idx_conversations_source;
ALTER TABLE conversations DROP COLUMN source;
