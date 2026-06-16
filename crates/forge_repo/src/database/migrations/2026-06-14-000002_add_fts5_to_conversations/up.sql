-- Create FTS5 virtual table for conversation search
-- This indexes both title and context content for full-text search
CREATE VIRTUAL TABLE IF NOT EXISTS conversations_fts USING fts5(
    conversation_id UNINDEXED,
    title,
    content,
    tokenize='porter'
);

-- Trigger to insert into FTS5 when a new conversation is created
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

-- Trigger to update FTS5 when a conversation is updated
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

-- Trigger to delete from FTS5 when a conversation is deleted
CREATE TRIGGER IF NOT EXISTS conversations_fts_delete
AFTER DELETE ON conversations
BEGIN
    DELETE FROM conversations_fts WHERE conversation_id = OLD.conversation_id;
END;

-- Populate the FTS5 table with existing conversations
INSERT INTO conversations_fts(conversation_id, title, content)
SELECT conversation_id, COALESCE(title, ''), COALESCE(context, '')
FROM conversations
WHERE context IS NOT NULL;
