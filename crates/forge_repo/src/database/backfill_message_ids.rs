use std::path::Path;

use anyhow::{Context as _, Result};
use chrono::Utc;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use forge_domain::MessageId;
use serde_json::Value;
use tracing::{info, warn};

use crate::database::schema::conversations;

/// Rows 100 per transaction; small enough that a lost compare-and-swap
/// re-reads negligible work, large enough to keep commit overhead down.
const BATCH_SIZE: i64 = 100;

/// Summary of a single backfill run. A fully-migrated DB reports
/// `updated == 0`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Report {
    pub scanned: usize,
    pub updated: usize,
    pub skipped: usize,
}

/// Walks every `conversations.context` blob and assigns a fresh `MessageId`
/// to any `MessageEntry` lacking one. Idempotent. Halts on JSON parse
/// failures so a corrupt row surfaces rather than being silently skipped.
pub(crate) fn run(conn: &mut SqliteConnection, database_path: Option<&Path>) -> Result<Report> {
    if let Some(path) = database_path {
        backup_db(path)?;
    }

    let mut report = Report::default();
    let mut offset = 0i64;

    loop {
        let rows: Vec<(String, String)> = conversations::table
            .filter(conversations::context.is_not_null())
            .order(conversations::conversation_id.asc())
            .limit(BATCH_SIZE)
            .offset(offset)
            .select((
                conversations::conversation_id,
                conversations::context.assume_not_null(),
            ))
            .load(conn)
            .context("failed to read conversations batch")?;

        if rows.is_empty() {
            break;
        }

        conn.transaction::<_, anyhow::Error, _>(|conn| {
            for (conv_id, original_blob) in &rows {
                report.scanned += 1;
                let backfilled = backfill_blob(original_blob).with_context(|| {
                    format!("corrupt context JSON in conversation {conv_id}")
                })?;
                let Some(new_blob) = backfilled else {
                    report.skipped += 1;
                    continue;
                };

                // Compare-and-swap: a concurrent writer that landed between
                // our read and this UPDATE invalidates the WHERE match;
                // `affected == 0` and we skip, leaving the winner's blob.
                let affected = diesel::update(conversations::table)
                    .filter(conversations::conversation_id.eq(conv_id))
                    .filter(conversations::context.eq(original_blob))
                    .set(conversations::context.eq(&new_blob))
                    .execute(conn)?;

                if affected == 1 {
                    report.updated += 1;
                } else {
                    report.skipped += 1;
                }
            }
            Ok(())
        })?;

        offset += BATCH_SIZE;
    }

    info!(
        scanned = report.scanned,
        updated = report.updated,
        skipped = report.skipped,
        "MessageId backfill migration complete"
    );

    Ok(report)
}

fn backup_db(path: &Path) -> Result<()> {
    if matches!(path.to_str(), Some(":memory:")) {
        return Ok(());
    }
    if !path.exists() {
        // Fresh DB with no file yet (first run); nothing to back up.
        return Ok(());
    }
    let ts = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let backup = path.with_extension(format!("pre-msgid-{ts}"));
    if let Err(err) = std::fs::copy(path, &backup) {
        // A missing-backup is non-fatal — we still want the migration to run,
        // but the operator should know the safety net failed.
        warn!(
            error = %err,
            target = %backup.display(),
            "failed to create pre-migration DB backup; proceeding without it",
        );
    } else {
        info!(backup = %backup.display(), "created pre-migration DB backup");
    }
    Ok(())
}

/// Returns `Some(new_blob)` when at least one message was rewritten,
/// `None` when the blob was already fully populated.
fn backfill_blob(blob: &str) -> Result<Option<String>> {
    let mut value: Value = serde_json::from_str(blob)?;
    let Some(messages) = value.get_mut("messages").and_then(|v| v.as_array_mut()) else {
        return Ok(None);
    };

    let mut changed = false;
    for msg in messages {
        let Some(obj) = msg.as_object_mut() else { continue };

        if obj.contains_key("id") {
            continue;
        }

        let fresh = serde_json::to_value(MessageId::new())?;
        if obj.contains_key("message") {
            // Wrapper form `{"message":..., "usage":...}` without `id`.
            obj.insert("id".to_string(), fresh);
        } else {
            // Direct form (bare `ContextMessageValueRecord`, e.g.
            // `{"text":{...}}`): rewrap as `{"id":..., "message":{...}}`.
            let inner = Value::Object(std::mem::take(obj));
            let mut wrapper = serde_json::Map::new();
            wrapper.insert("id".to_string(), fresh);
            wrapper.insert("message".to_string(), inner);
            *msg = Value::Object(wrapper);
        }
        changed = true;
    }

    if !changed {
        return Ok(None);
    }
    Ok(Some(serde_json::to_string(&value)?))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use diesel::r2d2::{ConnectionManager, Pool};
    use diesel_migrations::MigrationHarness;

    use super::*;
    use crate::database::pool::MIGRATIONS;

    #[derive(Debug)]
    struct BusyTimeoutCustomizer;

    impl diesel::r2d2::CustomizeConnection<SqliteConnection, diesel::r2d2::Error>
        for BusyTimeoutCustomizer
    {
        fn on_acquire(
            &self,
            conn: &mut SqliteConnection,
        ) -> std::result::Result<(), diesel::r2d2::Error> {
            // Without a busy_timeout, concurrent writers on `:memory:` fail
            // immediately with "database is locked"; with one, the loser of
            // the compare-and-swap waits long enough to retry its read.
            diesel::sql_query("PRAGMA busy_timeout = 5000;")
                .execute(conn)
                .map_err(diesel::r2d2::Error::QueryError)?;
            Ok(())
        }
    }

    fn new_conn() -> diesel::r2d2::PooledConnection<ConnectionManager<SqliteConnection>> {
        // `cache=shared` lets a single test optionally open extra connections
        // against the same in-memory DB; plain `:memory:` gives each
        // connection a private DB.
        let url = format!(
            "file:backfill-msgid-{}?mode=memory&cache=shared&uri=true",
            MessageId::new()
        );
        let manager = ConnectionManager::<SqliteConnection>::new(url);
        let pool = Pool::builder()
            .max_size(4)
            .connection_customizer(Box::new(BusyTimeoutCustomizer))
            .build(manager)
            .expect("build pool");
        let mut conn = pool.get().expect("get conn");
        conn.run_pending_migrations(MIGRATIONS)
            .expect("run migrations");
        conn
    }

    fn insert_conversation(conn: &mut SqliteConnection, id: &str, context: &str) {
        use diesel::RunQueryDsl;
        diesel::sql_query(format!(
            "INSERT INTO conversations (conversation_id, workspace_id, context, created_at) \
             VALUES ('{id}', 0, '{}', '2026-04-20 00:00:00')",
            context.replace('\'', "''"),
        ))
        .execute(conn)
        .expect("insert fixture row");
    }

    fn fetch_context(conn: &mut SqliteConnection, id: &str) -> String {
        conversations::table
            .filter(conversations::conversation_id.eq(id))
            .select(conversations::context.assume_not_null())
            .first(conn)
            .expect("fetch context")
    }

    /// Wrapper blob without `id` gets a fresh UUID, payload intact.
    #[test]
    fn test_backfill_wrapper_without_id() {
        let legacy = r#"{"messages":[{"message":{"text":{"role":"User","content":"hi"}},"usage":null}]}"#;
        let mut db = new_conn();
        insert_conversation(&mut db, "conv-1", legacy);

        let report = run(&mut db, None).unwrap();

        assert_eq!(report.scanned, 1);
        assert_eq!(report.updated, 1);
        let stored: Value = serde_json::from_str(&fetch_context(&mut db, "conv-1")).unwrap();
        let entry = &stored["messages"][0];
        assert!(entry.get("id").and_then(|v| v.as_str()).is_some());
        assert!(entry.get("message").is_some());
    }

    /// Direct-form blob (bare `{"text":{...}}`) is rewrapped as
    /// `{"id", "message"}` so the wrapper deserializer accepts it.
    #[test]
    fn test_backfill_rewraps_legacy_direct_form() {
        let legacy = r#"{"messages":[{"text":{"role":"User","content":"hi"}}]}"#;
        let mut db = new_conn();
        insert_conversation(&mut db, "conv-1", legacy);

        run(&mut db, None).unwrap();

        let stored: Value = serde_json::from_str(&fetch_context(&mut db, "conv-1")).unwrap();
        let entry = &stored["messages"][0];
        assert!(entry.get("id").and_then(|v| v.as_str()).is_some());
        assert!(entry.get("message").and_then(|m| m.get("text")).is_some());
    }

    /// A second run against an already-migrated DB rewrites nothing.
    #[test]
    fn test_backfill_is_idempotent() {
        let legacy = r#"{"messages":[{"message":{"text":{"role":"User","content":"hi"}}}]}"#;
        let mut db = new_conn();
        insert_conversation(&mut db, "conv-1", legacy);

        let first = run(&mut db, None).unwrap();
        assert_eq!(first.updated, 1);

        let second = run(&mut db, None).unwrap();
        assert_eq!(second.scanned, 1);
        assert_eq!(second.updated, 0);
        assert_eq!(second.skipped, 1);
    }

    /// A row with malformed JSON halts the migration, and the error names
    /// the conversation id so the operator can find and inspect the bad row.
    #[test]
    fn test_backfill_halts_on_corrupt_row() {
        let mut db = new_conn();
        insert_conversation(&mut db, "broken-row", "{not json");

        let err = run(&mut db, None).unwrap_err();
        let rendered = format!("{err:#}");
        assert!(
            rendered.contains("broken-row"),
            "error should name the offending conversation: {rendered}"
        );
    }

    /// Two concurrent runs both terminate cleanly; the winning CaS writes
    /// ids, the losing CaS skips.
    #[test]
    fn test_backfill_concurrent_runs_converge() {
        // Use a file-backed DB with WAL so two writers can actually race;
        // shared `:memory:` serialises everything and no CaS conflict can
        // arise.
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let path = tmp.path().to_owned();
        let url = path.to_string_lossy().to_string();
        let manager = ConnectionManager::<SqliteConnection>::new(url);
        let pool = Pool::builder()
            .max_size(4)
            .connection_customizer(Box::new(WalCustomizer))
            .build(manager)
            .expect("build pool");
        let mut setup = pool.get().unwrap();
        setup
            .run_pending_migrations(MIGRATIONS)
            .expect("run migrations");
        let legacy = r#"{"messages":[{"message":{"text":{"role":"User","content":"hi"}}}]}"#;
        insert_conversation(&mut setup, "conv-1", legacy);
        drop(setup);

        let barrier = Arc::new(std::sync::Barrier::new(2));
        let total_updated = Arc::new(AtomicUsize::new(0));
        let total_skipped = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..2 {
            let pool = pool.clone();
            let barrier = barrier.clone();
            let total_updated = total_updated.clone();
            let total_skipped = total_skipped.clone();
            handles.push(std::thread::spawn(move || {
                let mut conn = pool.get().unwrap();
                barrier.wait();
                let report = run(&mut conn, None).unwrap();
                total_updated.fetch_add(report.updated, Ordering::Relaxed);
                total_skipped.fetch_add(report.skipped, Ordering::Relaxed);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        // Exactly one CaS wins; the other sees the winner's blob and skips.
        assert_eq!(total_updated.load(Ordering::Relaxed), 1);
        assert_eq!(total_skipped.load(Ordering::Relaxed), 1);

        let mut verify = pool.get().unwrap();
        let stored: Value =
            serde_json::from_str(&fetch_context(&mut verify, "conv-1")).unwrap();
        let entry = &stored["messages"][0];
        assert!(entry.get("id").and_then(|v| v.as_str()).is_some());
    }

    #[derive(Debug)]
    struct WalCustomizer;

    impl diesel::r2d2::CustomizeConnection<SqliteConnection, diesel::r2d2::Error>
        for WalCustomizer
    {
        fn on_acquire(
            &self,
            conn: &mut SqliteConnection,
        ) -> std::result::Result<(), diesel::r2d2::Error> {
            // WAL + busy_timeout mirror production (see `SqliteCustomizer`)
            // and let two writers contend without immediate-lock errors.
            for pragma in [
                "PRAGMA journal_mode = WAL;",
                "PRAGMA busy_timeout = 5000;",
                "PRAGMA synchronous = NORMAL;",
            ] {
                diesel::sql_query(pragma)
                    .execute(conn)
                    .map_err(diesel::r2d2::Error::QueryError)?;
            }
            Ok(())
        }
    }
}
