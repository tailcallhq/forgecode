-- Revert to porter-only FTS5 (removes unicode61 + remove_diacritics=1).
--
-- WARNING: After down-migrating, search accuracy regresses for non-English text
-- and accented queries (cafe no longer matches cafe-accent). Existing data is
-- preserved; only the FTS5 tokenizer changes.
--
-- Populate the new (porter-only) table by invoking refresh_fts_index on
-- application startup after the migration runs.

DROP TABLE IF EXISTS conversations_fts;

CREATE VIRTUAL TABLE conversations_fts USING fts5(
    title,
    content,
    cwd,
    tokenize = 'porter'
);
