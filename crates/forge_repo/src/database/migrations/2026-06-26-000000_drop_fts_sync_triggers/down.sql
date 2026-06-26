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
