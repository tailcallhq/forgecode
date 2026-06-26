# Forge Database VACUUM & FTS Rebuild Tool — Design Spec

**Date**: 2026-06-26  
**Context**: Post-P2 (drop FTS triggers) + P2b (external-content FTS5) cleanup  
**Goal**: Safely reclaim ~2.76GB from `~/.forge.db` (currently 6.85GB)  
**Status**: Design specification (Rust binary; not yet implemented)

---

## Executive Summary

After P2 and P2b merge, the `~/.forge.db` SQLite database contains orphaned pages and fragmented FTS indices. A one-time VACUUM + FTS rebuild will reclaim ~40% disk space. However, VACUUM requires an **EXCLUSIVE lock**, meaning **NO process** (forge, forge-dev, or any open handle) can hold the database open.

This spec defines a **safety-first Rust binary** that:
- **Detects all processes holding the DB file** (using `lsof`)
- **Refuses to run** if any process is attached (exits non-zero with actionable message)
- **Never kills or signals any process** (absolute rule)
- **Backs up the database** before any writes (with disk-space preflight)
- **Runs the maintenance sequence** safely and idempotently
- **Reports progress and results** with before/after sizes and frames reclaimed

---

## Part 1: Hard Safety Rules (Central Design)

### 1.1 Process Hold Detection (Preflight)

The tool MUST detect all processes holding the database file **before attempting any operations**.

**Mechanism**: Use `lsof` (or `/proc` on Linux) to enumerate file descriptors.

```rust
/// Detect all processes holding the database file open.
/// Returns Vec of (pid, command) tuples.
/// 
/// Safety: This is read-only (lsof check), no side effects.
fn detect_open_handles(db_path: &Path) -> Result<Vec<(u32, String)>> {
    use std::process::Command;
    
    let output = Command::new("lsof")
        .arg("-t")  // Terse (PIDs only)
        .arg("--")
        .arg(db_path)
        .output()?;
    
    let mut pids = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Ok(pid) = line.trim().parse::<u32>() {
            // Get process name from /proc or ps
            if let Ok(name) = get_process_name(pid) {
                pids.push((pid, name));
            }
        }
    }
    Ok(pids)
}

/// Retrieve process name from /proc/<pid>/comm or ps
fn get_process_name(pid: u32) -> Result<String> {
    use std::fs;
    // Try /proc first (Linux)
    match fs::read_to_string(format!("/proc/{}/comm", pid)) {
        Ok(name) => Ok(name.trim().to_string()),
        Err(_) => {
            // Fallback: ps (macOS, BSD)
            let output = std::process::Command::new("ps")
                .arg("-p").arg(pid.to_string())
                .arg("-o").arg("comm=")
                .output()?;
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
    }
}

/// Preflight check: refuse if ANY process holds the DB open.
fn preflight_check(db_path: &Path) -> Result<()> {
    let handles = detect_open_handles(db_path)?;
    if !handles.is_empty() {
        eprintln!("ERROR: Cannot run VACUUM. {} process(es) hold {} open:", 
                  handles.len(), db_path.display());
        for (pid, cmd) in &handles {
            eprintln!("  PID {}: {}", pid, cmd);
        }
        eprintln!("\nClose all forge/forge-dev processes and try again.");
        std::process::exit(1);
    }
    Ok(())
}
```

**Behavior**:
- Runs at startup (before any writes)
- Lists all PIDs + command names holding the file
- **Exits with code 1 and actionable message** if any are found
- **Never** kills, signals, or terminates any process

### 1.2 No Kill, No Signal Rule

This is absolute and non-negotiable.

```rust
// ❌ FORBIDDEN:
// libc::kill(pid as i32, libc::SIGTERM);  // Never
// Command::new("kill").arg(...);           // Never
// std::process::Child::kill();              // Never

// ✅ ONLY allowed: read-only checks
lsof(db_path);  // Check if attached
ps::get_process_name(pid);  // Read process info
```

---

## Part 2: Backup & Disk-Space Preflight

### 2.1 Disk-Space Check

Before backing up, verify sufficient free space.

```rust
/// Check free disk space at the target location.
/// Refuse if < min_free_mb available.
fn check_disk_space(target_dir: &Path, min_free_mb: u64) -> Result<()> {
    use std::fs;
    
    let metadata = fs::metadata(target_dir)?;
    // On Unix, we can use statfs for accurate free space
    #[cfg(unix)]
    {
        use nix::sys::statvfs::statvfs;
        let stat = statvfs(target_dir)?;
        let free_bytes = stat.blocks_available() * stat.block_size();
        let free_mb = free_bytes / (1024 * 1024);
        
        if free_mb < min_free_mb {
            eprintln!("ERROR: Insufficient disk space.");
            eprintln!("  Required: {} MB for backup + VACUUM headroom",
                      min_free_mb);
            eprintln!("  Available: {} MB", free_mb);
            return Err("Disk space check failed".into());
        }
        eprintln!("✓ Disk check passed: {} MB free (need {} MB)", free_mb, min_free_mb);
    }
    Ok(())
}

/// Backup the database to a timestamped file.
/// Returns path to the backup.
fn backup_database(db_path: &Path, backup_dir: &Path) -> Result<PathBuf> {
    use std::fs;
    use chrono::Local;
    
    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let backup_name = format!(".forge_backup_{}.db", timestamp);
    let backup_path = backup_dir.join(&backup_name);
    
    eprintln!("Backing up {} → {}...", db_path.display(), backup_path.display());
    
    let start = std::time::Instant::now();
    fs::copy(db_path, &backup_path)?;
    
    let duration = start.elapsed();
    let size_gb = fs::metadata(&backup_path)?.len() as f64 / (1024.0 * 1024.0 * 1024.0);
    eprintln!("✓ Backup complete: {:.2} GB in {:.1}s", size_gb, duration.as_secs_f64());
    
    Ok(backup_path)
}
```

**Behavior**:
- Requires ~8GB free (6.85GB for DB + headroom for VACUUM working space)
- Exits with error if insufficient space
- Creates timestamped backup: `.forge_backup_20260626_143022.db`
- Reports size and duration

### 2.2 Skip-Backup Flag (With Warning)

```rust
fn backup_database_maybe(
    db_path: &Path,
    backup_dir: &Path,
    skip_backup: bool,
) -> Result<Option<PathBuf>> {
    if skip_backup {
        eprintln!("⚠️  WARNING: Skipping backup. If VACUUM fails, data may be corrupted.");
        eprintln!("   Proceed at your own risk. (Ctrl+C to cancel)");
        std::thread::sleep(std::time::Duration::from_secs(3));
        return Ok(None);
    }
    let backup = backup_database(db_path, backup_dir)?;
    Ok(Some(backup))
}
```

---

## Part 3: Maintenance Sequence (FTS Mode Detection & Rebuild)

### 3.1 Detect FTS Mode

Inspect `sqlite_master` to determine if the DB uses **external-content FTS5** (P2b) or **contentful FTS5** (pre-P2b).

```rust
/// FTS configuration mode detected from sqlite_master.
#[derive(Debug, Clone, Copy)]
enum FtsMode {
    /// External-content FTS5: `content='conversations'` in DDL
    /// Supports: 'rebuild', 'optimize' commands
    ExternalContent,
    
    /// Contentful FTS5 (pre-P2b): no content= clause
    /// Does NOT support 'rebuild'; must delete + repopulate
    Contentful,
    
    /// Unknown or mixed (should not occur in production)
    Unknown,
}

/// Detect FTS mode by inspecting the conversations_fts table DDL.
fn detect_fts_mode(conn: &rusqlite::Connection) -> Result<FtsMode> {
    let mut stmt = conn.prepare(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='conversations_fts'"
    )?;
    
    let ddl: String = stmt.query_row([], |row| row.get(0))?;
    
    if ddl.contains("content=") {
        eprintln!("✓ FTS mode detected: external-content (P2b)");
        Ok(FtsMode::ExternalContent)
    } else {
        eprintln!("✓ FTS mode detected: contentful (pre-P2b)");
        Ok(FtsMode::Contentful)
    }
}
```

**Why**: The rebuild strategy differs:
- **External-content** (P2b): Use `INSERT INTO conversations_fts(conversations_fts) VALUES('rebuild');`
- **Contentful** (pre-P2b): Use `DELETE FROM conversations_fts; INSERT INTO conversations_fts SELECT ...;` (as in P2's `refresh_fts_index`)

### 3.2 Maintenance Sequence

```rust
/// Full maintenance sequence: integrity → backup → vacuum → rebuild → optimize.
fn run_maintenance(
    db_path: &Path,
    fts_mode: FtsMode,
    dry_run: bool,
) -> Result<MaintenanceStats> {
    if dry_run {
        eprintln!("DRY RUN MODE: No writes will be executed.");
    }
    
    let mut conn = rusqlite::Connection::open(db_path)?;
    let mut stats = MaintenanceStats::default();
    
    // Step 1: Integrity check (quick_check)
    eprintln!("\n[1/5] Integrity check...");
    let before_pages = get_page_count(&conn)?;
    let before_size_gb = (before_pages * 4096) as f64 / (1024.0 * 1024.0 * 1024.0);
    eprintln!("  Pages: {}, Size: {:.2} GB", before_pages, before_size_gb);
    stats.pages_before = before_pages;
    stats.size_before_gb = before_size_gb;
    
    let integrity = quick_check(&conn)?;
    if !integrity.is_empty() {
        eprintln!("⚠️  Integrity warnings: {:?}", integrity);
    } else {
        eprintln!("✓ Integrity check passed");
    }
    
    // Step 2: Backup (already done in main flow)
    eprintln!("\n[2/5] Backup (already completed)");
    
    // Step 3: VACUUM
    eprintln!("\n[3/5] Running VACUUM...");
    if !dry_run {
        let start = std::time::Instant::now();
        conn.execute("VACUUM;", [])?;
        let duration = start.elapsed();
        eprintln!("✓ VACUUM complete in {:.1}s", duration.as_secs_f64());
    } else {
        eprintln!("(dry-run: VACUUM not executed)");
    }
    
    // Step 4: FTS Rebuild (mode-dependent)
    eprintln!("\n[4/5] FTS rebuild ({:?})...", fts_mode);
    if !dry_run {
        match fts_mode {
            FtsMode::ExternalContent => {
                // P2b: use built-in 'rebuild' command
                conn.execute(
                    "INSERT INTO conversations_fts(conversations_fts) VALUES('rebuild');",
                    []
                )?;
                eprintln!("✓ FTS rebuild (external-content) complete");
            }
            FtsMode::Contentful => {
                // Pre-P2b: delete + repopulate (from P2's refresh_fts_index)
                eprintln!("  Deleting FTS index...");
                conn.execute("DELETE FROM conversations_fts;", [])?;
                eprintln!("  Repopulating from source data...");
                // This would call the equivalent of P2's refresh_fts_index logic
                refresh_fts_index_contentful(&mut conn)?;
                eprintln!("✓ FTS rebuild (contentful) complete");
            }
            FtsMode::Unknown => {
                eprintln!("⚠️  Unknown FTS mode; skipping rebuild");
            }
        }
    } else {
        eprintln!("(dry-run: FTS rebuild not executed)");
    }
    
    // Step 5: FTS Optimize
    eprintln!("\n[5/5] FTS optimize...");
    if !dry_run {
        let start = std::time::Instant::now();
        conn.execute(
            "INSERT INTO conversations_fts(conversations_fts) VALUES('optimize');",
            []
        )?;
        let duration = start.elapsed();
        eprintln!("✓ FTS optimize complete in {:.1}s", duration.as_secs_f64());
    } else {
        eprintln!("(dry-run: FTS optimize not executed)");
    }
    
    // Step 6: Final integrity check
    eprintln!("\n[6/5] Final integrity check...");
    let after_pages = get_page_count(&conn)?;
    let after_size_gb = (after_pages * 4096) as f64 / (1024.0 * 1024.0 * 1024.0);
    let reclaimed_gb = before_size_gb - after_size_gb;
    let reclaimed_pct = (reclaimed_gb / before_size_gb) * 100.0;
    
    stats.pages_after = after_pages;
    stats.size_after_gb = after_size_gb;
    stats.reclaimed_gb = reclaimed_gb;
    stats.reclaimed_pct = reclaimed_pct;
    
    eprintln!("  Pages: {} (freed {} pages)", after_pages, before_pages - after_pages);
    eprintln!("  Size: {:.2} GB (reclaimed {:.2} GB, {:.1}%)", 
              after_size_gb, reclaimed_gb, reclaimed_pct);
    
    let final_integrity = quick_check(&conn)?;
    if !final_integrity.is_empty() {
        eprintln!("⚠️  Final integrity warnings: {:?}", final_integrity);
    } else {
        eprintln!("✓ Final integrity check passed");
    }
    
    Ok(stats)
}

/// Quick integrity check (PRAGMA quick_check)
fn quick_check(conn: &rusqlite::Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("PRAGMA quick_check;")?;
    let issues: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(issues)
}

/// Get current database page count.
fn get_page_count(conn: &rusqlite::Connection) -> Result<u64> {
    conn.query_row("PRAGMA page_count;", [], |row| row.get(0))
        .map_err(|e| e.into())
}

#[derive(Debug, Default)]
struct MaintenanceStats {
    pages_before: u64,
    pages_after: u64,
    size_before_gb: f64,
    size_after_gb: f64,
    reclaimed_gb: f64,
    reclaimed_pct: f64,
}
```

**Note on Rebuild Order**: 
- VACUUM is run **first** because it reassigns rowids and compacts pages
- FTS rebuild is run **after** VACUUM (the index needs the new rowid map)
- FTS optimize is run last (final polish)

---

## Part 4: Contentful FTS5 Repopulation (Cross-Reference to P2)

For pre-P2b databases, we need to repopulate the FTS index. This should reuse **P2's `refresh_fts_index` logic** (or a trimmed Rust equivalent).

```rust
/// Repopulate contentful FTS5 index by re-inserting from source table.
/// This is the Rust equivalent of P2's refresh_fts_index.
fn refresh_fts_index_contentful(conn: &mut rusqlite::Connection) -> Result<()> {
    let tx = conn.transaction()?;
    
    // Assume the FTS table is conversations_fts and source is conversations
    // The DDL defines which columns are indexed: typically (title, description, etc.)
    
    // Insert from source table (assumes conversations table exists)
    tx.execute(
        r#"
        INSERT INTO conversations_fts (rowid, title, description, body)
        SELECT id, title, description, body
        FROM conversations
        WHERE deleted_at IS NULL;
        "#,
        []
    )?;
    
    tx.commit()?;
    eprintln!("✓ FTS index repopulated from source");
    Ok(())
}
```

**Cross-reference**: This logic should be extracted from (or coordinated with) P2's `refresh_fts_index` implementation in the main codebase.

---

## Part 5: CLI Interface & Dry-Run

### 5.1 Command-Line Options

```rust
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "forge-vacuum")]
#[command(about = "Safely reclaim disk space from forge.db after P2/P2b merge")]
struct Args {
    /// Path to the .forge.db file
    #[arg(long, default_value = "~/.forge/.forge.db")]
    db_path: PathBuf,

    /// Directory to store backup (defaults to ~/.forge)
    #[arg(long)]
    backup_dir: Option<PathBuf>,

    /// Simulate the operation without writing to the database
    #[arg(long)]
    dry_run: bool,

    /// Skip backup step (⚠️ risky)
    #[arg(long)]
    skip_backup: bool,

    /// Minimum free disk space required (MB). Default: 8192 (8GB)
    #[arg(long, default_value = "8192")]
    min_free_mb: u64,

    /// Quiet mode (minimal output)
    #[arg(short, long)]
    quiet: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    
    // Expand ~ in paths
    let db_path = shellexpand::tilde(&args.db_path.to_string_lossy()).into_owned();
    let db_path = PathBuf::from(db_path);
    
    let backup_dir = args.backup_dir.unwrap_or_else(|| {
        let mut path = db_path.parent().unwrap().to_path_buf();
        path
    });
    
    eprintln!("forge-vacuum: database maintenance tool");
    eprintln!("DB path: {}", db_path.display());
    eprintln!("Backup dir: {}", backup_dir.display());
    
    // Preflight: detect open handles
    preflight_check(&db_path)?;
    
    // Disk space check
    check_disk_space(backup_dir.parent().unwrap_or(&backup_dir), args.min_free_mb)?;
    
    // Backup (unless --skip-backup)
    let backup_path = backup_database_maybe(&db_path, &backup_dir, args.skip_backup)?;
    
    // Detect FTS mode
    let conn = rusqlite::Connection::open(&db_path)?;
    let fts_mode = detect_fts_mode(&conn)?;
    drop(conn);
    
    // Run maintenance
    if args.dry_run {
        eprintln!("\n>>> DRY RUN: Would run maintenance sequence");
    }
    let stats = run_maintenance(&db_path, fts_mode, args.dry_run)?;
    
    // Report results
    eprintln!("\n=== MAINTENANCE COMPLETE ===");
    eprintln!("Before: {:.2} GB ({} pages)", stats.size_before_gb, stats.pages_before);
    eprintln!("After:  {:.2} GB ({} pages)", stats.size_after_gb, stats.pages_after);
    eprintln!("Reclaimed: {:.2} GB ({:.1}%)", stats.reclaimed_gb, stats.reclaimed_pct);
    if let Some(path) = backup_path {
        eprintln!("Backup: {}", path.display());
    }
    
    eprintln!("\n✓ Success!");
    Ok(())
}
```

### 5.2 Usage Examples

```bash
# Full maintenance (backup + vacuum + rebuild)
$ cargo run --release --bin forge-vacuum -- --db-path ~/.forge/.forge.db

# Dry run (preflight only, no writes)
$ cargo run --release --bin forge-vacuum -- --db-path ~/.forge/.forge.db --dry-run

# Skip backup (risky; only if space is constrained)
$ cargo run --release --bin forge-vacuum -- --db-path ~/.forge/.forge.db --skip-backup

# Custom backup directory
$ cargo run --release --bin forge-vacuum -- --db-path ~/.forge/.forge.db --backup-dir /mnt/backup

# Quiet mode
$ cargo run --release --bin forge-vacuum -- --db-path ~/.forge/.forge.db --quiet
```

---

## Part 6: Crate Structure & Dependencies

### 6.1 Placement & Organization

**Option A: Standalone binary in `tooling/` crate**
```
Phenotype/repos/forgecode-wts/
├── tooling/
│   ├── Cargo.toml
│   ├── src/
│   │   └── bin/
│   │       └── forge-vacuum/
│   │           ├── main.rs (CLI entry)
│   │           ├── lib.rs (core logic: preflight, vacuum, rebuild)
│   │           └── fts.rs (FTS mode detection & rebuild)
│   └── README.md
```

**Option B: Part of forge-cli workspace**
```
forge-cli/
├── Cargo.toml (workspace root)
├── forge-cli-core/
├── forge-cli/
├── forge-vacuum/  ← NEW
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       ├── lib.rs
│       ├── preflight.rs
│       └── fts.rs
```

**Recommendation**: **Option A** (standalone tooling binary). It's independent of forge-cli and can be run anytime.

### 6.2 Dependencies

```toml
# tooling/Cargo.toml
[package]
name = "forge-vacuum"
version = "0.1.0"
edition = "2021"
authors = ["Koosh Apari"]
description = "Safe maintenance tool for forge.db: VACUUM + FTS rebuild after P2/P2b"

[dependencies]
rusqlite = { version = "0.31", features = ["bundled", "chrono"] }
clap = { version = "4.5", features = ["derive"] }
chrono = "0.4"
shellexpand = "3.0"
nix = { version = "0.29", features = ["process", "fs"] }  # For statvfs, process info
anyhow = "1.0"
log = "0.4"
env_logger = "0.11"

[[bin]]
name = "forge-vacuum"
path = "src/bin/main.rs"

[profile.release]
opt-level = 3
lto = true
strip = true
```

**Why each dep**:
- `rusqlite`: SQLite access
- `clap`: CLI argument parsing
- `chrono`: Timestamped backups
- `shellexpand`: Handle `~` in paths
- `nix`: Disk space check (`statvfs`), process enumeration
- `anyhow`: Error handling
- `log` + `env_logger`: Structured logging (future)

---

## Part 7: Exit Codes & Error Messages

| Exit Code | Condition | Message |
|-----------|-----------|---------|
| 0 | Success | `✓ Success!` + stats |
| 1 | Process holds DB open | `ERROR: Cannot run VACUUM. N process(es) hold ... open:` + list PIDs |
| 2 | Insufficient disk space | `ERROR: Insufficient disk space. Required X MB, available Y MB` |
| 3 | Backup failed | `ERROR: Failed to back up database: ...` |
| 4 | Database corruption detected | `ERROR: Integrity check failed: ...` |
| 5 | Maintenance sequence failed | `ERROR: VACUUM/rebuild failed: ...` |

**All errors go to `stderr`, success goes to `stdout` (or `stderr` for progress).**

---

## Part 8: Safety & Idempotency

### 8.1 Idempotency

- Running the tool twice is safe (second run will find fewer pages to vacuum, smaller reclaim)
- Backup step: timestamped files, never overwrites
- FTS rebuild: `'rebuild'` is idempotent for external-content; delete+repopulate is idempotent for contentful

### 8.2 Failure Recovery

If the tool crashes mid-VACUUM:
- SQLite's WAL will ensure DB consistency (VACUUM commits atomically)
- If backup was not skipped, a rollback-ready copy exists at `~/.forge/.forge_backup_*.db`
- User can restore from backup and retry

If the tool crashes mid-FTS-rebuild:
- FTS table may be partially rebuilt (this is safe; rebuild is idempotent)
- Rerun the tool; it will complete the rebuild

### 8.3 Dry-Run Verification

Before running the tool on production:
```bash
$ forge-vacuum --db-path ~/.forge/.forge.db --dry-run
>>> DRY RUN: Would run maintenance sequence
[1/5] Integrity check...
  Pages: 1761280, Size: 6.85 GB
✓ Integrity check passed
[2/5] Backup (already completed)
[3/5] Running VACUUM...
(dry-run: VACUUM not executed)
[4/5] FTS rebuild...
(dry-run: FTS rebuild not executed)
[5/5] FTS optimize...
(dry-run: FTS optimize not executed)
[6/5] Final integrity check...
  Pages: (estimated 893824 after vacuum)
(dry-run estimate: would reclaim ~2.76 GB)
```

---

## Part 9: Implementation Roadmap

### Phase 1: Scaffolding (1–2 hours)
- Create `tooling/forge-vacuum/` crate
- Implement CLI skeleton (clap)
- Add `preflight.rs` with `lsof` integration

### Phase 2: Core Logic (2–3 hours)
- Implement `rusqlite` open + PRAGMA queries
- FTS mode detection
- VACUUM + rebuild + optimize sequence
- Error handling & logging

### Phase 3: Testing & Hardening (1–2 hours)
- Unit tests for FTS mode detection
- Integration tests on a test `.forge.db` copy
- Dry-run validation
- Edge cases: corrupted DB, missing tables, etc.

### Phase 4: Documentation & Release (30 min)
- README with usage examples
- Integration into forge's build system (optional: `forge maintenance vacuum`)
- Mention in P2/P2b PR descriptions

---

## Part 10: Cross-References & Dependencies

- **P2 Merge**: `refresh_fts_index()` logic (drop FTS triggers, switch to external-content FTS5)
- **P2b Merge**: External-content FTS5 implementation (sets `content=` in DDL)
- **This Tool**: Runs after both merges land; depends on knowing the FTS mode from P2b

---

## Part 11: Example Run Output

```
$ forge-vacuum --db-path ~/.forge/.forge.db
forge-vacuum: database maintenance tool
DB path: /home/user/.forge/.forge.db
Backup dir: /home/user/.forge

[PREFLIGHT]
✓ Disk check passed: 15240 MB free (need 8192 MB)
✓ No processes hold /home/user/.forge/.forge.db open

[BACKUP]
Backing up /home/user/.forge/.forge.db → /home/user/.forge/.forge_backup_20260626_143022.db...
✓ Backup complete: 6.85 GB in 45.3s

[MAINTENANCE]
[1/5] Integrity check...
  Pages: 1761280, Size: 6.85 GB
✓ Integrity check passed

[2/5] Backup (already completed)

[3/5] Running VACUUM...
✓ VACUUM complete in 32.1s

[4/5] FTS rebuild (ExternalContent)...
✓ FTS rebuild (external-content) complete

[5/5] FTS optimize...
✓ FTS optimize complete in 8.2s

[6/5] Final integrity check...
  Pages: 893824 (freed 867456 pages)
  Size: 3.49 GB (reclaimed 3.36 GB, 49.0%)
✓ Final integrity check passed

=== MAINTENANCE COMPLETE ===
Before: 6.85 GB (1761280 pages)
After:  3.49 GB (893824 pages)
Reclaimed: 3.36 GB (49.0%)
Backup: /home/user/.forge/.forge_backup_20260626_143022.db

✓ Success!
```

---

## Summary

This Rust binary is a **safety-first, idempotent maintenance tool** that:

1. ✅ **Detects and refuses** if any process holds the DB open (using `lsof`)
2. ✅ **Never kills** any process (absolute rule)
3. ✅ **Preflight disk-space check** (minimum 8GB free)
4. ✅ **Backs up** the full database (timestamped, ~45s for 6.85GB)
5. ✅ **Runs VACUUM** (atomic, reclaims ~40% disk space)
6. ✅ **Detects FTS mode** (external-content P2b vs. contentful pre-P2b)
7. ✅ **Rebuilds FTS** (mode-appropriate: `'rebuild'` or delete+repopulate)
8. ✅ **Optimizes FTS** (final polish)
9. ✅ **Reports results** (before/after sizes, reclaimed space, timing)
10. ✅ **Dry-run support** (preflight-only, no writes)

**Placement**: `Phenotype/repos/forgecode-wts/tooling/forge-vacuum/` (or integrate into forge-cli workspace).  
**LOC target**: ~400–500 lines (main.rs + lib.rs + fts.rs).  
**Dependencies**: rusqlite, clap, chrono, nix (for statvfs), anyhow.

---

**Next Step**: Once P2 and P2b merge, implement this binary. It should be battle-tested on a staging DB copy before users run it on production.
