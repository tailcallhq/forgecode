//! End-to-end split-DB integration test.
//!
//! Seeds a *legacy* DB on disk (the pre-split `.forge.db` with real rows —
//! exactly the state an existing install upgrades from) and lets the real
//! `forge` binary create + migrate the *write* DB (`.forge.writes.db`) at
//! startup via `FORGE_WRITE_DB_PATH` / `FORGE_LEGACY_DB_PATH`, then asserts
//! the `heliosdoctor` porcelain output:
//!
//! * `heliosdoctor --verbose` surfaces the UNION of both DBs — the legacy rows
//!   must remain visible even though the write DB is the new primary.
//! * `heliosdoctor --integrity-only` skips the COUNT queries entirely
//!   (`db_total=0`) and reports only the PRAGMA integrity result.
//!
//! This is the CI-level guard for the split-DB read path that the unit tests
//! cover in-process: the counts must agree when a real binary process opens
//! the seeded files.
//!
//! The legacy DB is intentionally seeded with the minimal column set the
//! stats queries touch (it is ATTACHed read-only and never migrated, so the
//! schema is stable); the write DB is left to the binary's own migrations.

use std::path::Path;
use std::process::Command;

use bstr::ByteSlice;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

/// Columns the stats queries touch. Aux tables (`messages`,
/// `context_compressions`, `checkpoints`) are absent on purpose:
/// `count_table` degrades to `None` for missing tables, which must not break
/// the report.
const CREATE_CONVERSATIONS: &str = "CREATE TABLE conversations (
    conversation_id TEXT PRIMARY KEY NOT NULL,
    title TEXT,
    workspace_id BIGINT NOT NULL,
    context TEXT,
    context_zstd BLOB,
    is_compressed INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP
)";

#[derive(Clone)]
struct SeedRow {
    id: &'static str,
    context: Option<&'static str>,
    context_zstd: Option<Vec<u8>>,
    is_compressed: bool,
}

fn seed_db(path: &Path, rows: &[SeedRow]) {
    let mut conn = SqliteConnection::establish(&path.display().to_string())
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    diesel::sql_query(CREATE_CONVERSATIONS)
        .execute(&mut conn)
        .unwrap_or_else(|e| panic!("create schema in {}: {e}", path.display()));

    for (i, row) in rows.iter().enumerate() {
        use diesel::sql_types::{Binary, Integer, Nullable, Text};
        let is_compressed = if row.is_compressed { 1 } else { 0 };
        diesel::sql_query(
            "INSERT INTO conversations \
             (conversation_id, workspace_id, context, context_zstd, is_compressed) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind::<Text, _>(format!("{}-{}", row.id, i))
        .bind::<Integer, _>(1_i32)
        .bind::<Nullable<Text>, _>(row.context)
        .bind::<Nullable<Binary>, _>(row.context_zstd.clone())
        .bind::<Integer, _>(is_compressed)
        .execute(&mut conn)
        .unwrap_or_else(|e| panic!("seed row in {}: {e}", path.display()));
    }
}

/// Runs `forge heliosdoctor ...` against the seeded split-DB layout.
///
/// The write DB path is NOT created by the test: the binary creates and
/// migrates it at startup, exactly like a fresh install.
fn run_doctor(write_db: &Path, legacy_db: &Path, args: &[&str]) -> String {
    let bin = env!("CARGO_BIN_EXE_forge");
    let output = Command::new(bin)
        .args(args)
        .env("FORGE_WRITE_DB_PATH", write_db)
        .env("FORGE_LEGACY_DB_PATH", legacy_db)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn forge binary");
    assert!(
        output.status.success(),
        "forge {:?} failed (status {:?})\nstdout:\n{}\nstderr:\n{}",
        args,
        output.status.code(),
        output.stdout.to_str_lossy(),
        output.stderr.to_str_lossy(),
    );
    assert!(
        write_db.exists(),
        "binary must have created and migrated the write DB: {}",
        write_db.display()
    );
    output.stdout.to_str_lossy().into_owned()
}

fn find_line<'a>(haystack: &'a str, key: &str) -> &'a str {
    haystack
        .lines()
        .find(|line| line.starts_with(&format!("{key}=")))
        .unwrap_or_else(|| panic!("missing porcelain line {key}= in:\n{haystack}"))
}

/// Legacy rows: 1 compressed, 1 uncompressed-agent, 1 uncompressed-user,
/// 1 empty. The write DB starts empty and is migrated by the binary.
fn seed_legacy_db(path: &Path) {
    seed_db(
        path,
        &[
            SeedRow {
                id: "legacy-compressed",
                context: None,
                context_zstd: Some(vec![0x00; 16]),
                is_compressed: true,
            },
            SeedRow {
                id: "legacy-agent",
                context: Some(r#"{"initiator":"agent","messages":[]}"#),
                context_zstd: None,
                is_compressed: false,
            },
            SeedRow {
                id: "legacy-user",
                context: Some(r#"{"initiator":"user","messages":[]}"#),
                context_zstd: None,
                is_compressed: false,
            },
            SeedRow {
                id: "legacy-empty",
                context: None,
                context_zstd: None,
                is_compressed: false,
            },
        ],
    );
}

#[test]
fn split_db_verbose_counts_union_of_legacy_and_write() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let write_db = tmp.path().join("write.db");
    let legacy_db = tmp.path().join("legacy.db");
    seed_legacy_db(&legacy_db);

    let out = run_doctor(
        &write_db,
        &legacy_db,
        &["heliosdoctor", "--verbose", "--porcelain"],
    );

    // write DB (migrated, empty) + legacy DB (4 seeded rows): the union must
    // surface the legacy rows even though the write DB is the new primary.
    assert_eq!(find_line(&out, "db_total"), "db_total=4");
    assert_eq!(find_line(&out, "db_compressed"), "db_compressed=1");
    assert_eq!(find_line(&out, "db_uncompressed"), "db_uncompressed=2");
    assert_eq!(find_line(&out, "db_empty"), "db_empty=1");
    assert_eq!(find_line(&out, "db_agent"), "db_agent=1");
    assert_eq!(find_line(&out, "db_oversized"), "db_oversized=0");
    assert_eq!(find_line(&out, "db_integrity"), "db_integrity=ok");
    assert_eq!(
        find_line(&out, "write_db"),
        &format!("write_db={}", write_db.display())
    );
    assert_eq!(
        find_line(&out, "legacy_db"),
        &format!("legacy_db={}", legacy_db.display())
    );
}

#[test]
fn split_db_integrity_only_skips_count_queries() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let write_db = tmp.path().join("write.db");
    let legacy_db = tmp.path().join("legacy.db");
    seed_legacy_db(&legacy_db);

    let out = run_doctor(
        &write_db,
        &legacy_db,
        &["heliosdoctor", "--integrity-only", "--porcelain"],
    );

    // PRAGMA-only: no COUNT queries, so every aggregate must be zero while
    // the integrity check still runs against both files.
    assert_eq!(find_line(&out, "db_total"), "db_total=0");
    assert_eq!(find_line(&out, "db_compressed"), "db_compressed=0");
    assert_eq!(find_line(&out, "db_uncompressed"), "db_uncompressed=0");
    assert_eq!(find_line(&out, "db_empty"), "db_empty=0");
    assert_eq!(find_line(&out, "db_agent"), "db_agent=0");
    assert_eq!(find_line(&out, "db_integrity"), "db_integrity=ok");
    assert_eq!(
        find_line(&out, "write_db"),
        &format!("write_db={}", write_db.display())
    );
    assert_eq!(
        find_line(&out, "legacy_db"),
        &format!("legacy_db={}", legacy_db.display())
    );
}
