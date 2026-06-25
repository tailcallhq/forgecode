//! SQLite-backed storage for the daemon.
//!
//! The daemon uses a single [`rusqlite::Connection`] guarded by a
//! [`parking_lot::Mutex`]. WAL journal mode is enabled at open time,
//! which permits concurrent readers from background tasks without
//! blocking the main accept loop. Because `rusqlite` is synchronous,
//! callers in async contexts should wrap invocations in
//! `tokio::task::spawn_blocking` (see `Server::drift_observe`).
//!
//! ## Schema
//!
//! * `agents`        — registered agent leases, refreshed on heartbeat.
//! * `drift_events`  — similarity alerts (source/target pair).
//! * `overrides`     — operator-applied suppressions keyed by alert id.
//!
//! All DDL is `IF NOT EXISTS`, so `Store::open` is idempotent and safe
//! to call on every daemon start.

use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use rusqlite::Connection;
use serde::Serialize;
use thiserror::Error;

/// Errors returned by [`Store`].
#[derive(Debug, Error)]
pub enum StoreError {
    /// SQLite returned an error.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// I/O failure (creating parent dir, opening file, etc.).
    #[error("io error on {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Schema migration failure.
    #[error("schema error: {0}")]
    Schema(String),
    /// Alert id was not found in `drift_events`.
    #[error("alert not found: {0}")]
    AlertNotFound(i64),
}

/// One row in the `drift_events` table. Also the payload of a
/// `drift.alert` notification.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DriftEvent {
    /// Row id in `drift_events`.
    pub id: i64,
    /// First agent whose prompt was observed.
    pub source_agent: String,
    /// Agent whose prompt overlapped with `source_agent`'s.
    pub target_agent: String,
    /// Similarity score in `[0.0, 1.0]`.
    pub similarity: f64,
    /// Lane tag supplied by the caller (e.g. `"plan"`, `"edit"`).
    pub lane: String,
    /// First 240 chars of the originating prompt.
    pub prompt_excerpt: String,
    /// Unix timestamp (seconds).
    pub created_at: i64,
    /// Unix timestamp (seconds). `None` until the alert is resolved.
    pub resolved_at: Option<i64>,
}

/// `Arc<Mutex<Connection>>` inner state. The connection is opened with
/// WAL mode and the schema is applied before the [`Store`] is
/// returned.
#[derive(Debug)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    /// Open or create the database at `db_path`. The parent directory
    /// is created if missing.
    pub fn open(db_path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| StoreError::Io {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }
        }
        let conn = Connection::open(db_path)?;
        enable_wal(&conn)?;
        apply_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Acquire the inner lock. Public so callers (e.g. the server's
    /// spawn_blocking wrappers) can drive transactions directly.
    pub fn conn(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
    }

    /// Record a drift event. Returns the row id assigned by SQLite.
    pub fn record_event(
        &self,
        source_agent: &str,
        target_agent: &str,
        similarity: f64,
        lane: &str,
        prompt_excerpt: &str,
    ) -> Result<i64, StoreError> {
        let now = unix_now();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO drift_events \
             (source_agent, target_agent, similarity, lane, prompt_excerpt, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                source_agent,
                target_agent,
                similarity,
                lane,
                prompt_excerpt,
                now
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// List the most recent unresolved alerts, newest first.
    pub fn list_recent_alerts(&self, limit: u32) -> Result<Vec<DriftEvent>, StoreError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, source_agent, target_agent, similarity, lane, \
                    prompt_excerpt, created_at, resolved_at \
             FROM drift_events \
             ORDER BY created_at DESC, id DESC \
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit as i64], row_to_event)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Apply an operator override. The alert's `resolved_at` is set
    /// to "now" and a row is inserted into `overrides` recording who
    /// applied it and why.
    ///
    /// Returns `Err(StoreError::AlertNotFound)` if no alert with the
    /// supplied id exists.
    pub fn apply_override(
        &self,
        alert_id: i64,
        reason: &str,
        actor: &str,
    ) -> Result<(), StoreError> {
        let now = unix_now();
        let conn = self.conn.lock();
        let updated = conn.execute(
            "UPDATE drift_events SET resolved_at = ?1 \
             WHERE id = ?2 AND resolved_at IS NULL",
            rusqlite::params![now, alert_id],
        )?;
        if updated == 0 {
            return Err(StoreError::AlertNotFound(alert_id));
        }
        conn.execute(
            "INSERT INTO overrides (alert_id, reason, actor, applied_at) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![alert_id, reason, actor, now],
        )?;
        Ok(())
    }

    /// Delete resolved alerts older than `days`. Returns the number
    /// of rows removed. Unresolved alerts are never pruned.
    pub fn prune_older_than(&self, days: u32) -> Result<usize, StoreError> {
        let cutoff = unix_now().saturating_sub(i64::from(days) * 86_400);
        let conn = self.conn.lock();
        let n = conn.execute(
            "DELETE FROM drift_events \
             WHERE resolved_at IS NOT NULL AND resolved_at < ?1",
            rusqlite::params![cutoff],
        )?;
        Ok(n)
    }

    /// Upsert an agent row. Used by `agent.register` and
    /// `agent.heartbeat`.
    pub fn upsert_agent(
        &self,
        agent_id: &str,
        pid: u32,
        label: &str,
        lane: &str,
        registered_at: i64,
        last_heartbeat: i64,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO agents \
             (id, pid, label, lane, registered_at, last_heartbeat) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(id) DO UPDATE SET \
                pid = excluded.pid, \
                label = excluded.label, \
                lane = excluded.lane, \
                last_heartbeat = excluded.last_heartbeat",
            rusqlite::params![
                agent_id,
                pid as i64,
                label,
                lane,
                registered_at,
                last_heartbeat
            ],
        )?;
        Ok(())
    }

    /// Delete the agent row. No-op if it does not exist.
    pub fn delete_agent(&self, agent_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM agents WHERE id = ?1", rusqlite::params![agent_id])?;
        Ok(())
    }
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<DriftEvent> {
    Ok(DriftEvent {
        id: row.get(0)?,
        source_agent: row.get(1)?,
        target_agent: row.get(2)?,
        similarity: row.get(3)?,
        lane: row.get(4)?,
        prompt_excerpt: row.get(5)?,
        created_at: row.get(6)?,
        resolved_at: row.get(7)?,
    })
}

fn enable_wal(conn: &Connection) -> Result<(), StoreError> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    Ok(())
}

fn apply_schema(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(SCHEMA_SQL)
        .map_err(|e| StoreError::Schema(format!("initial migration: {e}")))?;
    Ok(())
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS agents (
    id              TEXT PRIMARY KEY,
    pid             INTEGER NOT NULL,
    label           TEXT NOT NULL,
    lane            TEXT NOT NULL,
    registered_at   INTEGER NOT NULL,
    last_heartbeat  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_agents_heartbeat ON agents(last_heartbeat);

CREATE TABLE IF NOT EXISTS drift_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    source_agent    TEXT NOT NULL,
    target_agent    TEXT NOT NULL,
    similarity      REAL NOT NULL,
    lane            TEXT NOT NULL,
    prompt_excerpt  TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    resolved_at     INTEGER
);
CREATE INDEX IF NOT EXISTS idx_drift_events_created ON drift_events(created_at);
CREATE INDEX IF NOT EXISTS idx_drift_events_unresolved
    ON drift_events(created_at) WHERE resolved_at IS NULL;

CREATE TABLE IF NOT EXISTS overrides (
    alert_id        INTEGER NOT NULL REFERENCES drift_events(id),
    reason          TEXT NOT NULL,
    actor           TEXT NOT NULL,
    applied_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_overrides_alert ON overrides(alert_id);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_db() -> Store {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("drift.sqlite");
        // Leak the directory so the test can keep using the path.
        std::mem::forget(dir);
        Store::open(&path).expect("open")
    }

    #[test]
    fn schema_creates_all_three_tables() {
        let store = fresh_db();
        let conn = store.conn.lock();
        let names: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' \
                 AND name IN ('agents','drift_events','overrides') \
                 ORDER BY name",
            )
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(names, vec!["agents", "drift_events", "overrides"]);
    }

    #[test]
    fn wal_mode_enabled() {
        let store = fresh_db();
        let conn = store.conn.lock();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
    }

    #[test]
    fn record_event_returns_increasing_ids() {
        let store = fresh_db();
        let a = store
            .record_event("agent-a", "agent-b", 0.42, "plan", "hello")
            .unwrap();
        let b = store
            .record_event("agent-c", "agent-d", 0.99, "edit", "world")
            .unwrap();
        assert!(b > a);
    }

    #[test]
    fn list_recent_alerts_orders_newest_first() {
        let store = fresh_db();
        store.record_event("a", "b", 0.1, "p", "x").unwrap();
        store.record_event("c", "d", 0.2, "p", "y").unwrap();
        store.record_event("e", "f", 0.3, "p", "z").unwrap();
        let recent = store.list_recent_alerts(10).unwrap();
        assert_eq!(recent.len(), 3);
        assert!(recent[0].created_at >= recent[1].created_at);
        assert!(recent[1].created_at >= recent[2].created_at);
    }

    #[test]
    fn apply_override_resolves_alert() {
        let store = fresh_db();
        let id = store.record_event("a", "b", 0.5, "p", "x").unwrap();
        store.apply_override(id, "known false positive", "koosha").unwrap();
        let recent = store.list_recent_alerts(10).unwrap();
        assert_eq!(recent.len(), 1);
        assert!(recent[0].resolved_at.is_some());
    }

    #[test]
    fn apply_override_unknown_alert_errors() {
        let store = fresh_db();
        let err = store.apply_override(9999, "x", "y").unwrap_err();
        assert!(matches!(err, StoreError::AlertNotFound(9999)));
    }

    #[test]
    fn prune_older_than_removes_only_resolved() {
        let store = fresh_db();
        let a = store.record_event("a", "b", 0.1, "p", "x").unwrap();
        let _unresolved = store.record_event("c", "d", 0.2, "p", "y").unwrap();
        store.apply_override(a, "ok", "tester").unwrap();
        let n = store.prune_older_than(0).unwrap();
        assert_eq!(n, 1);
        let remaining = store.list_recent_alerts(10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].resolved_at.is_none());
    }

    #[test]
    fn agent_upsert_then_delete() {
        let store = fresh_db();
        store.upsert_agent("alpha", 4242, "tester", "plan", 100, 200).unwrap();
        // upsert again — must replace, not duplicate.
        store.upsert_agent("alpha", 4242, "tester", "edit", 100, 300).unwrap();
        let conn = store.conn.lock();
        let lane: String = conn
            .query_row("SELECT lane FROM agents WHERE id = 'alpha'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(lane, "edit");
        drop(conn);
        store.delete_agent("alpha").unwrap();
        let conn = store.conn.lock();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}