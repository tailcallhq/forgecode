# P2b: External-Content FTS5 Migration (Draft)

**Status:** DESIGN / DRAFT  
**Target:** Reclaim ~2.76 GB via FTS5 external-content mode  
**Scope:** DDL + query rewrites (NOT APPLIED)  
**Related:** P2 (drop sync triggers), P1 (wal_autocheckpoint)

---

## 1. Goal & Space Reclamation

### Problem

The current FTS5 configuration is **CONTENTFUL** (stores a full copy of indexed data):

```sql
CREATE VIRTUAL TABLE conversations_fts USING fts5(
    conversation_id UNINDEXED,
    title,
    content,  -- Full copy of context blob
    tokenize='porter'
);
```

- **conversations_fts_content table:** ~2.76 GB (duplicate of conversations.context)
- **conversations table:** Primary storage ~2.5 GB
- **Total footprint:** ~5.26 GB for the same content in two places

### Solution

Convert to **EXTERNAL-CONTENT** mode: store only FTS5 index metadata, fetch content on demand from the base table.

**Expected space savings:** ~2.76 GB (FTS content table eliminated)

---

## 2. Rowid Stability & Design Options

### The Problem: TEXT Primary Key & Implicit Rowid

```sql
CREATE TABLE conversations (
    conversation_id TEXT PRIMARY KEY NOT NULL,  -- NOT INTEGER
    ...
)
```

Because `conversation_id` is TEXT (not INTEGER PRIMARY KEY), SQLite creates an implicit, **unstable `rowid`**:

- **VACUUM:** Rewrites the table, reassigns ALL rowids
- **Deletes & reinserts:** Rowids can be reused or reassigned
- **Migration risk:** FTS index keyed on old rowids becomes stale

### Option A1: External-Content + Mandatory Rebuild After VACUUM

**Approach:**  
- Use external-content mode keyed on implicit `rowid`
- After every VACUUM, rebuild the entire FTS table
- Requires out-of-band rebuild logic or scheduled maintenance

**Pros:**
- No schema changes to `conversations` table
- No Diesel schema.rs update
- Minimal migration risk

**Cons:**
- FTS unavailable during rebuild (no lock-light operation)
- Explicit VACUUM + rebuild discipline required
- Operational toil; no automation guarantee

**Verdict:** ❌ High operational risk for a 6.85 GB database

---

### Option A2: Add Explicit `id INTEGER PRIMARY KEY` Surrogate (RECOMMENDED)

**Approach:**  
1. Add explicit `id INTEGER PRIMARY KEY AUTOINCREMENT` to `conversations` table
2. Use `content_rowid='id'` in external-content FTS5 definition
3. Rowid is now stable across VACUUM (tied to explicit column)
4. Drop all sync triggers (P2 work)

**Migration steps:**
1. Alter conversations table: add `id INTEGER PRIMARY KEY`
2. Drop old FTS table, create new external-content FTS
3. Create lightweight delete triggers (optional; can skip for P2 phase)
4. Rebuild FTS via out-of-band maintenance script (separate from migration)

**Pros:**
- ✅ Rowid stable across VACUUM
- ✅ No rebuild discipline required
- ✅ Clear separation: index stays valid
- ✅ Standard FTS5 pattern (Chromium, SQLite docs recommend this)

**Cons:**
- Schema change to conversations table
- Diesel schema.rs update
- requires rebuild after migration (deferred to maintenance window)

**Verdict:** ✅ **RECOMMENDED** — safe, standard, operational guarantee

---

## 3. Migration DDL (Option A2)

### Overview

**Three phases:**

1. **Alter base table** — add explicit `id INTEGER PRIMARY KEY` (migration, live, lock-light)
2. **Drop old FTS, create external-content FTS** — (migration, live, lock-light)
3. **Rebuild index** — VACUUM + rebuild deferred to maintenance window (separate process, not in migration)

---

### 3.1 New Migration: `2026-06-26-000000_external_fts5.sql`

#### Up (Apply)

```sql
-- Phase 1: Add explicit integer primary key to conversations
-- This makes rowid stable across VACUUM operations
-- NOTE: SQLite does NOT require explicit migrations for adding a primary key column
-- if the column is INTEGER PRIMARY KEY — it becomes an alias for the implicit rowid.
-- However, for clarity and future-proofing, we add it explicitly.

-- Step 1a: Create new conversations table with id column
-- (SQLite requires full table rebuild for schema changes)
CREATE TABLE conversations_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id TEXT NOT NULL UNIQUE,
    title TEXT,
    workspace_id BIGINT NOT NULL,
    context TEXT,
    created_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP,
    metrics TEXT,
    parent_id TEXT,
    source TEXT,
    cwd TEXT,
    message_count INTEGER
);

-- Step 1b: Copy all data from old table (preserves relative rowid ordering)
INSERT INTO conversations_new (
    id, conversation_id, title, workspace_id, context,
    created_at, updated_at, metrics, parent_id, source, cwd, message_count
)
SELECT rowid, conversation_id, title, workspace_id, context,
       created_at, updated_at, metrics, parent_id, source, cwd, message_count
FROM conversations;

-- Step 1c: Preserve the old table's indexes on new schema
CREATE INDEX IF NOT EXISTS idx_conversations_workspace ON conversations_new(workspace_id);
CREATE INDEX IF NOT EXISTS idx_conversations_parent ON conversations_new(parent_id);
CREATE INDEX IF NOT EXISTS idx_conversations_source ON conversations_new(source);
CREATE INDEX IF NOT EXISTS idx_conversations_created ON conversations_new(created_at);
CREATE INDEX IF NOT EXISTS idx_conversations_updated ON conversations_new(updated_at);
CREATE INDEX IF NOT EXISTS idx_conversations_cwd ON conversations_new(cwd);

-- Step 1d: Drop old table and its triggers
DROP TRIGGER IF EXISTS conversations_fts_insert;
DROP TRIGGER IF EXISTS conversations_fts_update;
DROP TRIGGER IF EXISTS conversations_fts_delete;
DROP TABLE conversations;

-- Step 1e: Rename new table to original name
ALTER TABLE conversations_new RENAME TO conversations;

-- Phase 2: Drop old contentful FTS5 table and recreate as external-content
-- This step runs LIVE and is lock-light (FTS table is virtual).

-- Step 2a: Drop the old FTS table (and its auto-generated content table)
DROP TABLE IF EXISTS conversations_fts;

-- Step 2b: Create new external-content FTS5 table
-- Columns match the base table columns we want to search:
--   - title: user-facing conversation name (searchable)
--   - content: indexed from conversations.context (blob content, searchable)
--   - cwd: working directory (optional, not indexed but available for metadata)
-- Note: conversation_id is NOT in FTS (external-content uses rowid for joining)
CREATE VIRTUAL TABLE conversations_fts USING fts5(
    title,
    content,
    cwd,
    tokenize='porter',
    content='conversations',
    content_rowid='id'
);

-- Step 2c: Populate the FTS index with existing data
-- This is the equivalent of INSERT in external-content mode:
-- FTS5 will read from the base table and build its index.
-- NOTE: This step is EXPENSIVE for large tables; it is deferred to a
-- maintenance window AFTER this migration completes (see section 5).
-- For now, we insert a NULL rebuild to mark FTS as stale.
-- INSERT INTO conversations_fts(rowid, title, content, cwd)
-- SELECT id, COALESCE(title, ''), COALESCE(context, ''), COALESCE(cwd, '')
-- FROM conversations
-- WHERE context IS NOT NULL;
--
-- DEFERRED: Rebuild runs separately via maintenance script (see section 5).

-- Step 2d: Recreate lightweight delete triggers (optional)
-- These maintain FTS consistency without the heavy write cost of the old triggers.
-- Option: SKIP this step if P2 is already dropping triggers entirely.
-- If enabled, replace OLD rowid reference with OLD.id:
--
-- CREATE TRIGGER IF NOT EXISTS conversations_fts_delete
-- AFTER DELETE ON conversations
-- BEGIN
--     DELETE FROM conversations_fts WHERE rowid = OLD.id;
-- END;

-- (End of migration up)
```

---

#### Down (Rollback)

```sql
-- ROLLBACK TO CONTENTFUL FTS5
-- This reverts to the pre-P2b state. Requires the old sync triggers to be re-added.

-- Step 1: Drop the new external-content FTS table
DROP TABLE IF EXISTS conversations_fts;

-- Step 2: Recreate the old contentful FTS5 table (with conversations_fts_content)
CREATE VIRTUAL TABLE conversations_fts USING fts5(
    conversation_id UNINDEXED,
    title,
    content,
    tokenize='porter'
);

-- Step 3: Rebuild from base table (same as original migration)
INSERT INTO conversations_fts(conversation_id, title, content)
SELECT conversation_id, COALESCE(title, ''), COALESCE(context, '')
FROM conversations
WHERE context IS NOT NULL;

-- Step 4: Recreate the old sync triggers (assumes they are needed for rollback)
-- NOTE: If P2 has already dropped these triggers, this step may fail.
-- Rollback will need to restore them from git history or previous migration.
CREATE TRIGGER IF NOT EXISTS conversations_fts_insert
AFTER INSERT ON conversations
BEGIN
    INSERT INTO conversations_fts(conversation_id, title, content)
    VALUES (
        NEW.conversation_id,
        COALESCE(NEW.title, ''),
        COALESCE(NEW.context, '')
    );
END;

CREATE TRIGGER IF NOT EXISTS conversations_fts_update
AFTER UPDATE ON conversations
BEGIN
    DELETE FROM conversations_fts WHERE conversation_id = OLD.conversation_id;
    INSERT INTO conversations_fts(conversation_id, title, content)
    VALUES (
        NEW.conversation_id,
        COALESCE(NEW.title, ''),
        COALESCE(NEW.context, '')
    );
END;

CREATE TRIGGER IF NOT EXISTS conversations_fts_delete
AFTER DELETE ON conversations
BEGIN
    DELETE FROM conversations_fts WHERE conversation_id = OLD.conversation_id;
END;

-- Step 5: Drop the id column from conversations (requires table rebuild)
-- NOTE: This is complex; rollback may require restoring from backup or manual recovery.
-- For now, we leave the id column in place and restore FTS functionality only.
-- A full rollback would need to re-DROP the id column and re-add the old schema.

-- (End of migration down)
```

---

## 4. Query Rewrites in `conversation_repo.rs`

### Key Change: `rowid` Joins

Currently, searches join by `conversation_id` column (because FTS is contentful):

```rust
// CURRENT (contentful FTS5)
let mut sql = String::from(
    "SELECT c.*, bm25(conversations_fts) AS rank_score \
     FROM conversations c \
     JOIN conversations_fts fts ON c.conversation_id = fts.conversation_id \
     WHERE conversations_fts MATCH ? \
       AND c.workspace_id = ? \
     ORDER BY rank_score",
);
```

**With external-content FTS, join by `rowid` (now = `id` column):**

```rust
// NEW (external-content FTS5)
let mut sql = String::from(
    "SELECT c.*, bm25(conversations_fts) AS rank_score \
     FROM conversations c \
     JOIN conversations_fts fts ON c.id = fts.rowid \
     WHERE conversations_fts MATCH ? \
       AND c.workspace_id = ? \
     ORDER BY rank_score",
);
```

**Location:** `/crates/forge_repo/src/conversation/conversation_repo.rs` line 303

---

### Snippet Query Rewrite

Currently, snippet column index is **2** (columns: conversation_id, title, content):

```rust
// CURRENT (contentful FTS5, 3 columns)
let sql = format!(
    "SELECT snippet(conversations_fts, 2, '[', ']', '…', {}) AS s \
     FROM conversations_fts \
     WHERE conversation_id = ? AND conversations_fts MATCH ?",
    token_count.min(256)
);
```

**With external-content FTS (new columns: title, content, cwd), content is now column index 1:**

```rust
// NEW (external-content FTS5, 3 columns: title, content, cwd)
let sql = format!(
    "SELECT snippet(conversations_fts, 1, '[', ']', '…', {}) AS s \
     FROM conversations_fts \
     WHERE rowid = (SELECT id FROM conversations WHERE conversation_id = ?) \
       AND conversations_fts MATCH ?",
    token_count.min(256)
);
```

**Alternative (direct rowid if stored in context):**

If the calling code knows the numeric `id`, the join simplifies:

```rust
// SIMPLEST (if id passed directly)
let sql = format!(
    "SELECT snippet(conversations_fts, 1, '[', ']', '…', {}) AS s \
     FROM conversations_fts \
     WHERE rowid = ? AND conversations_fts MATCH ?",
    token_count.min(256)
);
```

**Location:** `/crates/forge_repo/src/conversation/conversation_repo.rs` lines 351–356

---

## 5. Migration & Operations Sequence

### Prerequisite: P2 Completion

This migration **assumes P2 (drop sync triggers) is complete**. If not, coordinate timing:

1. **P2 runs first** — drop the 3 sync triggers (conversations_fts_insert, update, delete)
2. **P2b migration** — alter table, drop old FTS, create external-content FTS
3. **Maintenance window** — VACUUM + FTS rebuild (deferred)

---

### Migration Execution (Live, Lock-Light)

```
Target: 6.85 GB database
Estimated impact:
  - ALTER TABLE (table rebuild): ~20–30 min (lock-light WAL, no full VACUUM)
  - DROP old FTS: ~10 sec (virtual table drop)
  - CREATE external-content FTS: ~1 sec (index not populated yet)
  - Total: ~20–30 min
```

**Steps:**

1. **Backup** — snapshot database before migration
2. **Run migration up** — diesel CLI or manual execution in transaction
3. **Verify** — confirm old FTS gone, new FTS present (empty)
4. **Restart app** — reload Diesel schema.rs (schema change)
5. **Deploy query changes** — update conversation_repo.rs (line 303, 351–356)
6. **Test searches** — expect empty results until rebuild (see step 7)

---

### Maintenance Window (Separate Process)

**When:** Off-peak time (late night, weekend, or scheduled maintenance)  
**Duration:** ~45–60 min for 6.85 GB table

```bash
# Pseudo-code (not in migration)

sqlite3 /path/to/database.db <<'EOF'
  -- Full VACUUM to compact WAL and reassign stable rowids
  VACUUM;

  -- Rebuild external-content FTS from base table
  -- (Full index scan; slow but done once)
  INSERT INTO conversations_fts(rowid, title, content, cwd)
  SELECT id, COALESCE(title, ''), COALESCE(context, ''), COALESCE(cwd, '')
  FROM conversations
  WHERE context IS NOT NULL;

  -- Optimize FTS index (reduces file size)
  INSERT INTO conversations_fts(conversations_fts) VALUES('optimize');
EOF
```

**Why separate?**
- VACUUM cannot run in a transaction (Diesel migrations run in tx)
- Rebuild is expensive; better run offline or in scheduled maintenance
- Gives ops team control over downtime window

---

## 6. Risk Table

| Risk | Severity | Mitigation | Notes |
|------|----------|-----------|-------|
| **Rowid drift after VACUUM** | HIGH | Explicit `id INTEGER PRIMARY KEY` ties rowid to column, stable across VACUUM | Must add id column; no rollback risk once applied |
| **Search-unavailable window** | MEDIUM | Rebuild deferred to maintenance; accept ~few hours until rebuild | FTS index is empty until rebuild; queries return 0 results (not errors) |
| **Snippet column off-by-one** | MEDIUM | Verify column index in rewrite (title=0, content=1, cwd=2) | Manual testing required; easy to fix if wrong |
| **Diesel schema.rs mismatch** | HIGH | Update schema.rs with new `id` column; regenerate if needed | `diesel migration run` auto-updates; verify before deploy |
| **Sync triggers collision (P2)** | MEDIUM | Coordinate P2 completion before P2b; don't recreate old triggers | P2b does NOT recreate heavyweight delete triggers |
| **Missed query updates** | HIGH | Search in 2 places: line 303 (rowid join), lines 351–356 (snippet index) | Review all 2 locations; run full test suite |
| **Rebuild failure mid-way** | MEDIUM | Rebuild is idempotent (INSERT OR REPLACE); can retry | If rebuild interrupted, run again in next maintenance window |
| **Large table rebuild duration** | MEDIUM | 6.85 GB → ~45–60 min rebuild; schedule off-peak | Monitor first run; adjust estimate for future |

---

## 7. Diesel Schema & Build Implications

### schema.rs Update

Current (before P2b):
```rust
diesel::table! {
    conversations (conversation_id) {
        conversation_id -> Text,
        title -> Nullable<Text>,
        workspace_id -> BigInt,
        context -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
        metrics -> Nullable<Text>,
        parent_id -> Nullable<Text>,
        source -> Nullable<Text>,
        #[sql_name = "cwd"]
        cwd -> Nullable<Text>,
        #[sql_name = "message_count"]
        message_count -> Nullable<Integer>,
    }
}
```

After P2b (with explicit `id` PRIMARY KEY):
```rust
diesel::table! {
    conversations (id) {  // PRIMARY KEY changes from conversation_id to id
        id -> Integer,
        conversation_id -> Text,
        title -> Nullable<Text>,
        workspace_id -> BigInt,
        context -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
        metrics -> Nullable<Text>,
        parent_id -> Nullable<Text>,
        source -> Nullable<Text>,
        #[sql_name = "cwd"]
        cwd -> Nullable<Text>,
        #[sql_name = "message_count"]
        message_count -> Nullable<Integer>,
    }
}
```

### Regeneration

```bash
cd crates/forge_repo
diesel migration run --database-url 'sqlite://test.db'
diesel print-schema > src/database/schema.rs
```

**Or:** Manual edit if regeneration fails (rare).

---

## 8. Implementation Checklist

### Migration Phase

- [ ] Create migration SQL file: `2026-06-26-000000_external_fts5`
  - [ ] Phase 1: Add `id INTEGER PRIMARY KEY` via table rebuild
  - [ ] Phase 1: Copy data + indexes
  - [ ] Phase 1: Drop old table + sync triggers
  - [ ] Phase 2: Drop old FTS table
  - [ ] Phase 2: Create external-content FTS (empty)
- [ ] Verify migration rolls forward and backward
- [ ] Test on a copy of production DB

### Code Changes

- [ ] Update `conversation_repo.rs` line 303: join by `c.id = fts.rowid`
- [ ] Update `conversation_repo.rs` lines 351–356: snippet column 1 (content), join by rowid
- [ ] Regenerate or manually update `schema.rs`
- [ ] Run full test suite: `cargo test --workspace`
- [ ] Manual test search + snippet on test database

### Operations

- [ ] Document rebuild procedure (separate doc or README)
- [ ] Create maintenance script: `scripts/rebuild_fts_after_vacuum.sh`
- [ ] Schedule rebuild window (off-peak)
- [ ] Monitor rebuild performance on staging DB
- [ ] Prepare rollback plan (keep backup; know down.sql)

---

## 9. Deployment Order

1. **P2 (if not done):** Drop sync triggers, land in main
2. **P2b code review:** Query rewrites + schema.rs change
3. **Deploy:** New code + migration (live, lock-light)
4. **Verify:** Searches return 0 results (expected, FTS empty until rebuild)
5. **Maintenance window:** Run rebuild script (VACUUM + rebuild)
6. **Verify:** Searches return results again

---

## References

- [SQLite FTS5 External Content Tables](https://www.sqlite.org/fts5.html#external_content_tables)
- [SQLite INTEGER PRIMARY KEY](https://www.sqlite.org/lang_createtable.html#rowid)
- P2 design: P2_DROP_SYNC_TRIGGERS.md (sibling doc)
- P1 design: P1_WAL_AUTOCHECKPOINT.md (sibling doc)

