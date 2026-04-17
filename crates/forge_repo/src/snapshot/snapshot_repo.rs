use std::sync::Arc;

use diesel::prelude::*;
use forge_domain::{ConversationId, Snapshot, SnapshotMetadataRepository, UserInputId};

use super::snapshot_record::SnapshotRecord;
use crate::database::schema::snapshot_metadata;
use crate::database::DatabasePool;

/// SQLite-backed repository for snapshot metadata.
///
/// Persists a row into the `snapshot_metadata` table for every file snapshot
/// created, enabling bulk-undo queries by `UserInputId`.
pub struct SnapshotMetadataRepositoryImpl {
    pool: Arc<DatabasePool>,
}

impl SnapshotMetadataRepositoryImpl {
    /// Creates a new `SnapshotMetadataRepositoryImpl` backed by the given pool.
    pub fn new(pool: Arc<DatabasePool>) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl SnapshotMetadataRepository for SnapshotMetadataRepositoryImpl {
    async fn insert_snapshot_metadata(
        &self,
        snapshot: &Snapshot,
        snap_file_path: String,
    ) -> anyhow::Result<()> {
        let mut conn = self.pool.get_connection()?;
        let record = SnapshotRecord::new(snapshot, snap_file_path);
        diesel::insert_into(snapshot_metadata::table)
            .values(&record)
            .on_conflict(snapshot_metadata::snapshot_id)
            .do_nothing()
            .execute(&mut conn)?;
        Ok(())
    }

    async fn find_snapshots_by_user_input_id(
        &self,
        user_input_id: UserInputId,
    ) -> anyhow::Result<Vec<(String, String)>> {
        let mut conn = self.pool.get_connection()?;
        let rows: Vec<(String, String)> = snapshot_metadata::table
            .filter(snapshot_metadata::user_input_id.eq(user_input_id.to_string()))
            .filter(snapshot_metadata::undone_at.is_null())
            .select((snapshot_metadata::file_path, snapshot_metadata::snap_file_path))
            .load(&mut conn)?;
        Ok(rows)
    }

    async fn find_snapshots_by_conversation_id(
        &self,
        conversation_id: ConversationId,
    ) -> anyhow::Result<Vec<(String, String, String)>> {
        let mut conn = self.pool.get_connection()?;
        let rows: Vec<(String, String, String)> = snapshot_metadata::table
            .filter(snapshot_metadata::conversation_id.eq(conversation_id.to_string()))
            .filter(snapshot_metadata::undone_at.is_null())
            .order(snapshot_metadata::created_at.desc())
            .select((
                snapshot_metadata::user_input_id,
                snapshot_metadata::file_path,
                snapshot_metadata::snap_file_path,
            ))
            .load(&mut conn)?;
        Ok(rows)
    }

    async fn mark_snapshots_undone(&self, user_input_id: UserInputId) -> anyhow::Result<()> {
        let mut conn = self.pool.get_connection()?;
        let now = chrono::Utc::now().naive_utc();
        diesel::update(snapshot_metadata::table)
            .filter(snapshot_metadata::user_input_id.eq(user_input_id.to_string()))
            .filter(snapshot_metadata::undone_at.is_null())
            .set(snapshot_metadata::undone_at.eq(now))
            .execute(&mut conn)?;
        Ok(())
    }
}
