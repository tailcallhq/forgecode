-- Phenotype-org (2026-06-26): drop synchronous FTS maintenance triggers.
-- They re-tokenized the full `context` blob inline on every conversation
-- update, holding the WAL writer lock and causing 'database is locked'
-- under concurrent forge processes. FTS is now refreshed out-of-band
-- (see ConversationRepository::refresh_fts_index). The contentful
-- conversations_fts table itself is unchanged.
DROP TRIGGER IF EXISTS conversations_fts_insert;
DROP TRIGGER IF EXISTS conversations_fts_update;
DROP TRIGGER IF EXISTS conversations_fts_delete;
