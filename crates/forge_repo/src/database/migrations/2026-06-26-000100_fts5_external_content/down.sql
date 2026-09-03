-- Rollback to contentful FTS5 (pre-P2b state).
-- Re-creates the old table with the same schema it had before external-content conversion.

DROP TABLE IF EXISTS conversations_fts;
CREATE VIRTUAL TABLE conversations_fts USING fts5(
    conversation_id UNINDEXED,
    title,
    content,
    cwd,
    tokenize='porter'
);
