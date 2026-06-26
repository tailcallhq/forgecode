-- Revert to external-content FTS5 (broken for compressed rows, but restores
-- space savings). WARNING: Search will not find compressed rows until
-- refresh_fts_index is fixed to work with external-content mode.

DROP TABLE IF EXISTS conversations_fts;

CREATE VIRTUAL TABLE conversations_fts USING fts5(
    title, context, cwd,
    content='conversations', content_rowid='rowid', tokenize='porter'
);
