-- P0 (v3): Add cwd + message_count to conversations; extend FTS5 to index cwd.
--
-- `cwd` lets the session selector group and filter by working directory, and
-- lets FTS5 search match when the user types a project-name fragment.
--
-- `message_count` is a denormalised count of `context.messages` written at
-- upsert time. Storing it as a column (rather than computing it from the
-- serialised Context blob at read time) keeps the selector fast — the
-- selector can build its display row from the row columns alone and never
-- has to deserialize the full context.
--
-- The two columns are nullable so the migration is non-blocking: existing
-- rows have `NULL` until they are next touched by `upsert_conversation_ref`
-- (which now writes both fields), at which point they get backfilled.
--
-- The new FTS5 column lets the user search by cwd fragment (e.g. "forgecode")
-- without touching the heavyweight `content` column. We use
-- `INSERT INTO conversations_fts(conversations_fts, ...)` to rebuild the row
-- and an `INSERT INTO conversations_fts(conversations_fts)` no-op to keep
-- the trigger simple. Both the insert and update triggers are rewritten to
-- include the new column.

ALTER TABLE conversations ADD COLUMN cwd TEXT;
ALTER TABLE conversations ADD COLUMN message_count INTEGER;

-- Recreate the FTS5 virtual table with a `cwd` column.
--
-- The original `conversations_fts` (from 2026-06-14-000002) is dropped and
-- recreated. SQLite FTS5 doesn't support `ALTER TABLE ... ADD COLUMN`, so
-- drop + recreate is the canonical migration. Existing rows are reindexed
-- in the same statement.
DROP TABLE IF EXISTS conversations_fts;

CREATE VIRTUAL TABLE IF NOT EXISTS conversations_fts USING fts5(
    conversation_id UNINDEXED,
    title,
    content,
    cwd,
    tokenize='porter'
);

-- Rebuild the FTS5 index from the current contents of `conversations`.
-- `cwd` is the new column; `content` is the serialised Context blob
-- (already indexed previously).
INSERT INTO conversations_fts(conversation_id, title, content, cwd)
SELECT conversation_id, COALESCE(title, ''), COALESCE(context, ''), COALESCE(cwd, '')
FROM conversations;

-- Drop the old triggers (if present) and recreate them to write the new
-- `cwd` column as well.
DROP TRIGGER IF EXISTS conversations_fts_insert;
DROP TRIGGER IF EXISTS conversations_fts_update;
DROP TRIGGER IF EXISTS conversations_fts_delete;

CREATE TRIGGER IF NOT EXISTS conversations_fts_insert
AFTER INSERT ON conversations
BEGIN
    INSERT INTO conversations_fts(conversation_id, title, content, cwd)
    VALUES (
        NEW.conversation_id,
        COALESCE(NEW.title, ''),
        COALESCE(NEW.context, ''),
        COALESCE(NEW.cwd, '')
    );
END;

CREATE TRIGGER IF NOT EXISTS conversations_fts_update
AFTER UPDATE ON conversations
BEGIN
    DELETE FROM conversations_fts WHERE conversation_id = OLD.conversation_id;
    INSERT INTO conversations_fts(conversation_id, title, content, cwd)
    VALUES (
        NEW.conversation_id,
        COALESCE(NEW.title, ''),
        COALESCE(NEW.context, ''),
        COALESCE(NEW.cwd, '')
    );
END;

CREATE TRIGGER IF NOT EXISTS conversations_fts_delete
AFTER DELETE ON conversations
BEGIN
    DELETE FROM conversations_fts WHERE conversation_id = OLD.conversation_id;
END;

-- P0-3 (round 3): partial composite index supporting the "cwd fragment" filter.
--
-- The selector's cwd-grouped lookup is `workspace_id = ? AND cwd = ?`,
-- ordered by recency. A composite (workspace_id, cwd) lets SQLite walk
-- the index in workspace order and skip rows that belong to a different
-- workspace. The partial `context IS NOT NULL` predicate matches the
-- selector's application filter, so the index only stores rows that the
-- list paths can ever return.
CREATE INDEX IF NOT EXISTS idx_conversations_workspace_cwd
    ON conversations(workspace_id, cwd)
    WHERE context IS NOT NULL;

-- P0-3 (round 3): partial composite index supporting the "by message count"
-- sort. The selector sorts by `message_count DESC` for the "by turns" pick.
-- A composite (workspace_id, message_count DESC) is the canonical pattern
-- for "top N by count" queries.
CREATE INDEX IF NOT EXISTS idx_conversations_workspace_message_count
    ON conversations(workspace_id, message_count DESC)
    WHERE context IS NOT NULL;
