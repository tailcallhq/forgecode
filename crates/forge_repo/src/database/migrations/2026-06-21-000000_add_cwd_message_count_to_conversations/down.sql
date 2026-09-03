-- Reverse of 2026-06-21-000000_add_cwd_message_count_to_conversations/up.sql.
--
-- This migration unwinds in the opposite order of `up.sql`:
--   1. Drop the new triggers
--   2. Drop the new composite indexes
--   3. Recreate the FTS5 virtual table without the `cwd` column
--   4. Recreate the original 3 triggers (insert/update/delete)
--   5. Drop the `cwd` and `message_count` columns

DROP TRIGGER IF EXISTS conversations_fts_insert;
DROP TRIGGER IF EXISTS conversations_fts_update;
DROP TRIGGER IF EXISTS conversations_fts_delete;

DROP INDEX IF EXISTS idx_conversations_workspace_cwd;
DROP INDEX IF EXISTS idx_conversations_workspace_message_count;

DROP TABLE IF EXISTS conversations_fts;

CREATE VIRTUAL TABLE IF NOT EXISTS conversations_fts USING fts5(
    conversation_id UNINDEXED,
    title,
    content,
    tokenize='porter'
);

INSERT INTO conversations_fts(conversation_id, title, content)
SELECT conversation_id, COALESCE(title, ''), COALESCE(context, '')
FROM conversations
WHERE context IS NOT NULL;

CREATE TRIGGER IF NOT EXISTS conversations_fts_insert
AFTER INSERT ON conversations
BEGIN
    INSERT INTO conversations_fts(conversation_id, title, content)
    VALUES (
        NEW.conversation_id,
        COALESCE(NEW.title, ''),
        COALESCE(NEW.context, '')
    );
END;

CREATE TRIGGER IF NOT EXISTS conversations_fts_update
AFTER UPDATE ON conversations
BEGIN
    DELETE FROM conversations_fts WHERE conversation_id = OLD.conversation_id;
    INSERT INTO conversations_fts(conversation_id, title, content)
    VALUES (
        NEW.conversation_id,
        COALESCE(NEW.title, ''),
        COALESCE(NEW.context, '')
    );
END;

CREATE TRIGGER IF NOT EXISTS conversations_fts_delete
AFTER DELETE ON conversations
BEGIN
    DELETE FROM conversations_fts WHERE conversation_id = OLD.conversation_id;
END;

-- SQLite does not support DROP COLUMN before 3.35 (the version pinned in
-- Cargo.lock for this workspace predates 3.35). To make the down migration
-- reversible on the supported SQLite versions, the columns are left in
-- place; a manual `ALTER TABLE conversations DROP COLUMN cwd` and
-- `... DROP COLUMN message_count` would be required on a SQLite 3.35+ host.
-- This is a known limitation of the older pinned SQLite and is acceptable
-- for the down migration path (which is admin-only and rarely run).
