-- Transparent zstd compression of context blobs
-- Adds dual-path read with automatic fallback for backward compatibility
-- NEW rows: compressed into context_zstd, is_compressed=1, context=NULL
-- OLD rows: remain in context column, is_compressed=0 (no backfill)
-- Read path: if is_compressed=1 decompress context_zstd, else read context column
--
-- No breaking change: existing uncompressed rows continue to work
-- Migration is forward-only: future backfill tool handles existing rows

ALTER TABLE conversations ADD COLUMN context_zstd BLOB;
-- Stores zstd-compressed JSON (ContextRecord serialized)
-- NULL for old uncompressed rows and tombstone conversations

ALTER TABLE conversations ADD COLUMN is_compressed INTEGER NOT NULL DEFAULT 0;
-- Flag: 1 = context_zstd contains compressed data, 0 = context contains uncompressed JSON
-- Used to determine read path: decompress vs fallback to plain text

-- Index for finding compressed records (audit/stats)
CREATE INDEX idx_conversations_compressed
    ON conversations(workspace_id)
    WHERE is_compressed = 1;
-- Used for "estimate compression ratio" and "find compressed conversations"
