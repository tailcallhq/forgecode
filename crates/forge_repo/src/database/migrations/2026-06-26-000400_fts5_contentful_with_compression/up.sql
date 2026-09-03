-- Fix FTS5 to work with compressed rows.
--
-- The previous external-content FTS5 table read from the `context` column by name.
-- However, compressed rows have context=NULL and data in context_zstd.
-- This caused FTS5 to silently miss compressed rows.
--
-- SOLUTION: Revert to CONTENTFUL FTS5 (which stores its own copy of indexed columns).
-- This trades a modest space cost (FTS _content table becomes a searchable copy)
-- against correctness: both compressed and uncompressed rows are indexed.
--
-- The base conversations.context is still compressed (zstd on disk), so the PRIMARY
-- savings remain. The FTS _content copy is searchable but does not further compress.
-- This is pragmatic: FTS5 CONTENTFUL is the simplest correct design.

-- Drop the broken external-content FTS5 table
DROP TABLE IF EXISTS conversations_fts;

-- Create CONTENTFUL FTS5: stores its own indexed copy
CREATE VIRTUAL TABLE conversations_fts USING fts5(
    title,
    content,
    cwd,
    tokenize='porter'
);

-- Table is created EMPTY. Application-side refresh_fts_index will populate it
-- with decompressed context from both compressed and uncompressed rows.
