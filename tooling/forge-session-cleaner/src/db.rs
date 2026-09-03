use anyhow::{anyhow, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::path::Path;

pub struct RowRecord {
    pub id: String,
    pub parent_id: Option<String>,
    pub context: Value,
    pub context_bytes: usize,
}

pub fn open_immutable(path: &Path) -> Result<Connection> {
    // Use mode=ro, NOT immutable=1. The live DB is in WAL mode with a large
    // uncheckpointed WAL; immutable=1 ignores the -wal file and reads the main
    // db alone, which yields an inconsistent snapshot SQLite reports as
    // "database disk image is malformed". mode=ro reads the WAL too and is the
    // correct safe read-only mode against a DB with concurrent writers.
    let uri = format!("file:{}?mode=ro", path.display());
    Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(Into::into)
}

pub fn load_rows(conn: &Connection) -> Result<Vec<RowRecord>> {
    let mut stmt = conn.prepare(
        "SELECT conversation_id, parent_id, context FROM conversations ORDER BY conversation_id",
    )?;
    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let parent_id: Option<String> = row.get(1)?;
        // context can be NULL (empty/aborted sessions) — treat as empty JSON null.
        let context_text: Option<String> = row.get(2)?;
        let context_text = context_text.unwrap_or_default();
        let context: Value = if context_text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&context_text).unwrap_or(Value::Null)
        };
        Ok(RowRecord {
            id,
            parent_id,
            context_bytes: context_text.len(),
            context,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    if out.is_empty() {
        return Err(anyhow!("no rows loaded"));
    }
    Ok(out)
}

