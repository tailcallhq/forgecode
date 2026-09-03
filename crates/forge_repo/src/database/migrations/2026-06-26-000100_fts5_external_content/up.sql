-- Phenotype-org (2026-06-26): convert conversations_fts to external-content to drop the ~2.76GB duplicate copy.
-- P2b implements Option A1 (implicit-rowid external content, simplest).
-- This migration:
--   1. Drops the old contentful FTS5 table (which auto-creates conversations_fts_content with ~2.76GB)
--   2. Creates a new external-content FTS5 table that reads from conversations base table
--   3. Leaves the index empty; rebuild is deferred to maintenance window (requires VACUUM for rowid stability)

DROP TABLE IF EXISTS conversations_fts;
CREATE VIRTUAL TABLE conversations_fts USING fts5(
    title, context, cwd,
    content='conversations', content_rowid='rowid', tokenize='porter'
);
-- NOTE: external-content reads source columns BY NAME, so the fts columns are named to match
-- conversations' actual columns: title, context, cwd. No triggers (P2 removed them; refresh stays
-- out-of-band). Table starts EMPTY; search returns empty until refresh_fts_index runs 'rebuild'
-- (deferred to forge-vacuum / background refresh). The 'rebuild after VACUUM' rule is required
-- for rowid stability — forge-vacuum already does this.
