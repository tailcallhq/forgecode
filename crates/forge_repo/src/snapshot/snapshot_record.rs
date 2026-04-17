use chrono::NaiveDateTime;

use crate::database::schema::snapshot_metadata;

/// Database record for snapshot metadata
#[derive(Debug, Clone, diesel::Queryable, diesel::Selectable, diesel::Insertable)]
#[diesel(table_name = snapshot_metadata)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub(super) struct SnapshotRecord {
    pub snapshot_id: String,
    pub user_input_id: String,
    pub conversation_id: String,
    pub file_path: String,
    pub snap_file_path: String,
    pub created_at: NaiveDateTime,
    pub undone_at: Option<NaiveDateTime>,
}

impl SnapshotRecord {
    /// Converts a domain `Snapshot` into a `SnapshotRecord` for persistence.
    ///
    /// # Arguments
    /// * `snapshot` - The domain snapshot to persist.
    /// * `snap_file_path` - The absolute path to the `.snap` file on disk.
    pub fn new(snapshot: &forge_domain::Snapshot, snap_file_path: String) -> Self {
        Self {
            snapshot_id: snapshot.id.to_string(),
            user_input_id: snapshot.user_input_id.to_string(),
            conversation_id: snapshot.conversation_id.to_string(),
            file_path: snapshot.path.clone(),
            snap_file_path,
            created_at: chrono::DateTime::from_timestamp(
                snapshot.timestamp.as_secs() as i64,
                snapshot.timestamp.subsec_nanos(),
            )
            .unwrap_or_else(chrono::Utc::now)
            .naive_utc(),
            undone_at: None,
        }
    }
}
