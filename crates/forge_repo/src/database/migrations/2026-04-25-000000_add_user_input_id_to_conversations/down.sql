-- SQLite does not support DROP COLUMN before 3.35.0,
-- so we recreate the table without the user_input_id column.
CREATE TABLE conversations_backup (
    conversation_id TEXT PRIMARY KEY NOT NULL,
    title           TEXT,
    workspace_id    BIGINT NOT NULL,
    context         TEXT,
    created_at      TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at      TIMESTAMP,
    metrics         TEXT
);

INSERT INTO conversations_backup
    SELECT conversation_id, title, workspace_id, context, created_at, updated_at, metrics
    FROM conversations;

DROP TABLE conversations;

ALTER TABLE conversations_backup RENAME TO conversations;

CREATE INDEX IF NOT EXISTS idx_conversations_workspace_id
    ON conversations(workspace_id);
