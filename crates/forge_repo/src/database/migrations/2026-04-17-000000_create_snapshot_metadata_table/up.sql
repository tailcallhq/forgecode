CREATE TABLE IF NOT EXISTS snapshot_metadata (
    snapshot_id    TEXT PRIMARY KEY NOT NULL,
    user_input_id  TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    file_path      TEXT NOT NULL,
    snap_file_path TEXT NOT NULL,
    created_at     TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_snapshot_metadata_user_input_id
    ON snapshot_metadata(user_input_id);

CREATE INDEX IF NOT EXISTS idx_snapshot_metadata_conversation_id
    ON snapshot_metadata(conversation_id);
