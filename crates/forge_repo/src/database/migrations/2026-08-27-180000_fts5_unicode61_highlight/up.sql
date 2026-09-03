-- Add unicode61 tokenizer with remove_diacritics=1 to conversations_fts.
--
-- The existing FTS5 setup (migration 2026-06-26-000400) uses just `porter`
-- which is English-only and does NOT fold diacritics. This migration upgrades
-- the tokenizer to `unicode61 "remove_diacritics 1"`, which:
--   - Tokenizes non-English text per Unicode rules (unicode61)
--   - Folds diacritics so cafe, cafe-accent, and cafe-diacritic all match
--
-- NOTE on tokenizer choice:
--   SQLite FTS5 supports specifying exactly one tokenizer name per virtual
--   table. Combining porter + unicode61 natively is not supported; it would
--   require a custom Rust tokenizer. We pick unicode61 (better international
--   coverage, includes basic English tokenization via Unicode rules) over
--   porter (English-only stemming like running -> run).
--
--   If English stemming is critical, a follow-up migration can ship a custom
--   tokenizer wrapper that chains porter inside unicode61 (or vice versa).
--   Until then, unicode61 is the right default for a multilingual codebase.
--
-- Matches the target design intent in docs/requirements.md:52-58 (M3 spec).
-- PRESERVES the contentful-mode pattern from migration 2026-06-26-000400 so
-- compressed rows (is_compressed=1, context=NULL, context_zstd BLOB) continue
-- to be indexed by application-side refresh_fts_index after zstd decompression.
-- Does NOT reintroduce triggers (migration 2026-06-26-000000 dropped them to
-- fix WAL-lock contention; refresh_fts_index at startup remains the population
-- path).
--
-- PREREQUISITES:
--   - SQLite >= 3.44 (the bundled libsqlite3-sys in this crate is 3.51.x)
--   - Application must call refresh_fts_index after this migration completes,
--     either:
--     (a) explicitly on daemon startup (recommended; minimal first-search cost)
--     (b) implicitly via the first :search or /fts-optimize invocation
--
-- INDEX COST:
--   FTS5 has no ALTER TABLE for the tokenizer, so we drop + recreate the
--   virtual table. The existing index shadow content is lost; rebuild via
--   refresh_fts_index in Rust land.
--
-- ROW FORMAT PRESERVED:
--   conversations_fts(title, content, cwd) - same 3 indexed columns as before.
--   Column order matters: search_conversations JOINs on rowid (same),
--   snippet(col_idx, ...) uses content at column index 1 (same).

-- Drop the old porter-only FTS5 table.
DROP TABLE IF EXISTS conversations_fts;

-- Recreate with unicode61 + remove_diacritics=1.
-- unicode61 handles Unicode text per Unicode Standard Annex #29 (word
-- boundaries); remove_diacritics=1 folds accents so users searching
-- "cafe" find "cafe-accent" rows.
--
-- The tokenize value is unquoted-space-separated form (the most basic
-- FTS5 syntax). The prior attempt used the quoted form
--   tokenize = 'unicode61 "remove_diacritics 1"'
-- which bundled SQLite 3.51.x rejected with "parse error in tokenize
-- directive" - apparently the FTS5 tokenizer parser in this build does
-- not consume the inner double-quoted arg correctly. The unquoted form
-- works on every SQLite version that supports unicode61 (>= 3.27).
CREATE VIRTUAL TABLE conversations_fts USING fts5(
    title,
    content,
    cwd,
    tokenize='unicode61 remove_diacritics 1'
);

-- Table is created EMPTY. Application-side refresh_fts_index will populate it
-- with decompressed context from both compressed and uncompressed rows
-- (mirrors the population pattern set by migration 2026-06-26-000400).
--
-- refresh_fts_index lives in crates/forge_repo/src/conversation/conversation_repo.rs
-- and must be invoked once after this migration runs. Recommended call sites:
--   1. DatabasePool::build_pool after run_pending_migrations (crates/forge_repo/src/database/pool.rs:395)
--   2. forge_dbd server startup after open_writer_connection (crates/forge_dbd/src/server.rs:514)
