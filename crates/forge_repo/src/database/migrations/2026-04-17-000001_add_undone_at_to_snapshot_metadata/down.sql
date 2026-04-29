-- SQLite does not support DROP COLUMN before 3.35.0,
-- so we recreate the table without the undone_at column.
CREATE TABLE snapshot_metadata_backup (
    snapshot_id    TEXT PRIMARY KEY NOT NULL,
    user_input_id  TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    file_path      TEXT NOT NULL,
    snap_file_path TEXT NOT NULL,
    created_at     TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO snapshot_metadata_backup
    SELECT snapshot_id, user_input_id, conversation_id, file_path, snap_file_path, created_at
    FROM snapshot_metadata;

DROP TABLE snapshot_metadata;

ALTER TABLE snapshot_metadata_backup RENAME TO snapshot_metadata;

CREATE INDEX IF NOT EXISTS idx_snapshot_metadata_user_input_id
    ON snapshot_metadata(user_input_id);

CREATE INDEX IF NOT EXISTS idx_snapshot_metadata_conversation_id
    ON snapshot_metadata(conversation_id);
