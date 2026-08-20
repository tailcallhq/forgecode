use crate::classifier::{classify_first_user_message, first_user_message, Class};
use crate::db::{load_rows, open_immutable};
use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

pub fn run(db_path: &Path, apply: bool, tier: u8, yes: bool) -> Result<()> {
    let conn = open_immutable(db_path)?;
    let rows = load_rows(&conn)?;
    let mut by_id: HashMap<&String, &Option<String>> = HashMap::new();
    for r in &rows {
        by_id.insert(&r.id, &r.parent_id);
    }

    let mut records = Vec::new();
    let mut delete_tier1: HashSet<String> = HashSet::new();
    let mut delete_tier2: HashSet<String> = HashSet::new();
    let mut human = 0usize;
    let mut indeterminate = 0usize;
    let mut tier2_additional = 0usize;
    let mut reclaim1 = 0usize;
    let mut reclaim2 = 0usize;
    let mut path_mismatch = 0usize;

    for row in &rows {
        let msg = first_user_message(&row.context);
        let missing_path = msg.is_none();
        let (class, reason, snippet) = match msg.as_deref() {
            Some(m) => {
                let c = classify_first_user_message(m);
                let snippet = m.chars().take(200).collect::<String>();
                (c.class, c.reason, snippet)
            }
            None => (Class::Indeterminate, "missing first-user path".to_string(), String::new()),
        };
        if missing_path {
            path_mismatch += 1;
        }
        match class {
            Class::DeleteTier1 => {
                delete_tier1.insert(row.id.clone());
                delete_tier2.insert(row.id.clone());
                reclaim1 += row.context_bytes;
                reclaim2 += row.context_bytes;
            }
            Class::DeleteTier2 => {
                delete_tier2.insert(row.id.clone());
                tier2_additional += 1;
                reclaim2 += row.context_bytes;
            }
            Class::Human => human += 1,
            Class::Indeterminate => indeterminate += 1,
        }
        records.push((row.id.clone(), class, reason, snippet));
    }

    let mut child_only_tier1: HashSet<String> = HashSet::new();
    for row in &rows {
        if delete_tier1.contains(&row.id) {
            if let Some(parent) = &row.parent_id {
                if delete_tier1.contains(parent) {
                    child_only_tier1.insert(row.id.clone());
                }
            }
        }
    }

    let mut final_delete_tier1: HashSet<String> = HashSet::new();
    for row in &rows {
        if delete_tier1.contains(&row.id)
            && row
                .parent_id
                .as_ref()
                .is_none_or(|p| delete_tier1.contains(p))
        {
            final_delete_tier1.insert(row.id.clone());
        }
    }
    let final_tier1_count = final_delete_tier1.len();

    println!("DRY-RUN ONLY - no deletion performed");
    println!("rows: {}", rows.len());
    println!(
        "counts: KEEP={} DELETE-tier1={} would-be-DELETE-tier2-additional={} human={} indeterminate={}",
        rows.len() - final_tier1_count - tier2_additional,
        final_tier1_count,
        tier2_additional,
        human,
        indeterminate
    );
    println!("path_check_mismatches: {}", path_mismatch);
    println!("reclaimable_tier1_bytes: {}", reclaim1);
    println!("reclaimable_tier1_plus_2_bytes: {}", reclaim2);
    println!("predicate: tier1 starts-with/contains AI markers after stripping <task>; tier2 broader AI markers or >800 chars formal imperative; human short informal lowercase-start without AI markers; indeterminate keep; children delete only if parent is in delete set");
    println!("delete-mode requested tier={} (ignored because dry-run)", tier);

    println!("examples_delete_tier1:");
    for (_, class, reason, snippet) in records.iter().filter(|r| r.1 == Class::DeleteTier1).take(15) {
        println!("- {:?} | {} | {}", class, reason, first_150(snippet));
    }
    println!("examples_keep:");
    for (_, class, reason, snippet) in records.iter().filter(|r| r.1 == Class::Human || r.1 == Class::Indeterminate).take(15) {
        println!("- {:?} | {} | {}", class, reason, first_150(snippet));
    }
    println!("borderline:");
    for (_, class, reason, snippet) in records.iter().filter(|r| r.1 != Class::DeleteTier1).take(20) {
        println!("- {:?} | {} | {}", class, reason, first_200(snippet));
    }

    if !apply {
        return Ok(());
    }

    // Build the delete set for the requested tier.
    // tier1 = final_delete_tier1 (high-confidence AI, child-cascade-safe).
    // tier2 = tier1 PLUS the broader tier2 rows, with the same child-cascade
    // guard (only delete a child if its parent is also in the delete set).
    let delete_set: HashSet<String> = if tier >= 2 {
        let mut s = HashSet::new();
        for row in &rows {
            if delete_tier2.contains(&row.id)
                && row
                    .parent_id
                    .as_ref()
                    .is_none_or(|p| delete_tier2.contains(p))
            {
                s.insert(row.id.clone());
            }
        }
        s
    } else {
        final_delete_tier1.clone()
    };

    println!();
    println!("=== APPLY (tier {}) — {} conversations selected ===", tier, delete_set.len());

    if !yes {
        println!("--apply given WITHOUT --yes: this is a confirm preview. Re-run with --yes to delete.");
        return Ok(());
    }

    apply_delete(db_path, &delete_set)
}

/// Safely delete the selected conversations: refuse if any process holds the
/// DB, require disk headroom, back up first, then delete in a transaction.
fn apply_delete(db_path: &Path, delete_set: &HashSet<String>) -> Result<()> {
    // 1) SAFETY GATE: refuse if any process holds the db (or -wal/-shm) open.
    let pids = processes_holding(db_path);
    if !pids.is_empty() {
        return Err(anyhow!(
            "refusing to delete: {} process(es) hold the database open (pids: {}). \
             Close forge and retry. (No process was killed.)",
            pids.len(),
            pids.join(", ")
        ));
    }

    if delete_set.is_empty() {
        println!("nothing to delete.");
        return Ok(());
    }

    // 2) DISK CHECK: need room for a full backup of the (large) db.
    let db_bytes = fs::metadata(db_path).map(|m| m.len()).unwrap_or(0);
    let free = free_bytes(db_path)?;
    if free < db_bytes + 1_073_741_824 {
        return Err(anyhow!(
            "refusing: need ~{} GB free for backup, only {} GB available",
            (db_bytes / 1_073_741_824) + 1,
            free / 1_073_741_824
        ));
    }

    // 3) BACKUP first.
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = db_path.with_extension(format!("db.backup-{}", stamp));
    fs::copy(db_path, &backup)
        .map_err(|e| anyhow!("backup failed ({}): {}", backup.display(), e))?;
    println!("backed up to {}", backup.display());

    // 4) DELETE in a single transaction on a read-write connection.
    let mut conn = rusqlite::Connection::open(db_path)?;
    conn.execute_batch("PRAGMA busy_timeout = 30000;")?;
    let tx = conn.transaction()?;
    let mut deleted = 0usize;
    {
        let mut stmt = tx.prepare("DELETE FROM conversations WHERE conversation_id = ?1")?;
        for id in delete_set {
            deleted += stmt.execute([id])?;
        }
    }
    tx.commit()?;

    println!("deleted {} conversations (backup: {})", deleted, backup.display());
    println!("note: run forge-vacuum in a quiet window to reclaim the freed pages on disk.");
    Ok(())
}

/// Return the PIDs of processes holding any of db / db-wal / db-shm open.
/// Uses `lsof -t`; never signals or kills anything.
fn processes_holding(db_path: &Path) -> Vec<String> {
    let mut pids = HashSet::new();
    for suffix in ["", "-wal", "-shm"] {
        let target = format!("{}{}", db_path.display(), suffix);
        if let Ok(out) = Command::new("lsof").arg("-t").arg("--").arg(&target).output() {
            // clippy::disallowed_methods (from_utf8_lossy): lsof -t emits only
            // ASCII PIDs; no bstr dep is warranted for this standalone tool.
            #[allow(clippy::disallowed_methods)]
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                let p = line.trim();
                if !p.is_empty() {
                    pids.insert(p.to_string());
                }
            }
        }
    }
    let mut v: Vec<String> = pids.into_iter().collect();
    v.sort();
    v
}

/// Free bytes on the filesystem containing `path`, via `df -k`.
fn free_bytes(path: &Path) -> Result<u64> {
    let dir = path.parent().unwrap_or(path);
    let out = Command::new("df").arg("-k").arg(dir).output()?;
    // clippy::disallowed_methods (from_utf8_lossy): df -k emits ASCII; no bstr
    // dep is warranted for this standalone tool.
    #[allow(clippy::disallowed_methods)]
    let text = String::from_utf8_lossy(&out.stdout);
    // Second line, 4th column = available 1K-blocks.
    let avail_k: u64 = text
        .lines()
        .nth(1)
        .and_then(|l| l.split_whitespace().nth(3))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow!("could not parse df output"))?;
    Ok(avail_k * 1024)
}

fn first_150(s: &str) -> String {
    s.chars().take(150).collect()
}

fn first_200(s: &str) -> String {
    s.chars().take(200).collect()
}
