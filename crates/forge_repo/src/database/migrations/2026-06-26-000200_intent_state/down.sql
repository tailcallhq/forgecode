-- ADR-103: Intent-gated semantic pruning rollback
-- NOTE: SQLite < 3.35.0 cannot DROP COLUMN directly.
-- This migration performs a best-effort cleanup:
-- 1. Drop indexes (safe, reversible)
-- 2. ALTERs to drop columns (skipped on older SQLite versions)
-- If migration fails due to SQLite version, manual cleanup is required:
--   PRAGMA table_info(conversations) to list columns
--   Create new table without intent_state columns, copy data, swap tables

DROP INDEX IF EXISTS idx_conversations_intent_pending;
DROP INDEX IF EXISTS idx_conversations_intent_verified;
DROP INDEX IF EXISTS idx_conversations_memory_id;

-- SQLite 3.35.0+ supports DROP COLUMN; earlier versions must use table rebuild
-- If your SQLite is older, comment out the next 4 lines and perform manual cleanup
ALTER TABLE conversations DROP COLUMN IF EXISTS intent_state;
ALTER TABLE conversations DROP COLUMN IF EXISTS extracted_at;
ALTER TABLE conversations DROP COLUMN IF EXISTS memory_id;
ALTER TABLE conversations DROP COLUMN IF EXISTS intent_hash;
