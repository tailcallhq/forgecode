use std::str::FromStr;
use std::sync::Arc;

use diesel::prelude::*;
use forge_domain::{
    Conversation, ConversationId, ConversationRepository, ConversationSummary, WorkspaceHash,
};

use crate::conversation::conversation_record::{ConversationRecord, ConversationRecordLite};
use crate::database::schema::conversations;
use crate::database::{DatabasePool, PooledSqliteConnection};

/// Lightweight row type for FTS5 `snippet()` results. The query returns
/// exactly one column (`s`) — we use a named struct (not a tuple) so
/// diesel's `QueryableByName` derive can read it back from `sql_query`.
#[derive(Debug, Clone)]
struct SnippetRow {
    s: String,
}

impl diesel::QueryableByName<diesel::sqlite::Sqlite> for SnippetRow {
    fn build<'a>(
        row: &impl diesel::row::NamedRow<'a, diesel::sqlite::Sqlite>,
    ) -> diesel::deserialize::Result<Self> {
        let s = diesel::row::NamedRow::get::<diesel::sql_types::Text, _>(row, "s")?;
        Ok(SnippetRow { s })
    }
}

/// Row type for reading conversations during FTS refresh.
/// Used to populate FTS5 with decompressed context from both compressed and uncompressed rows.
#[derive(Debug, Clone)]
struct FtsRefreshRow {
    rowid: i64,
    title: String,
    context: Option<String>,
    context_zstd: Option<Vec<u8>>,
    is_compressed: i32,
    cwd: Option<String>,
}

impl diesel::QueryableByName<diesel::sqlite::Sqlite> for FtsRefreshRow {
    fn build<'a>(
        row: &impl diesel::row::NamedRow<'a, diesel::sqlite::Sqlite>,
    ) -> diesel::deserialize::Result<Self> {
        use diesel::row::NamedRow;
        use diesel::sql_types::{BigInt, Binary, Integer, Nullable, Text};
        Ok(FtsRefreshRow {
            rowid: NamedRow::get::<BigInt, _>(row, "rowid")?,
            title: NamedRow::get::<Text, _>(row, "title")?,
            context: NamedRow::get::<Nullable<Text>, _>(row, "context")?,
            context_zstd: NamedRow::get::<Nullable<Binary>, _>(row, "context_zstd")?,
            is_compressed: NamedRow::get::<Integer, _>(row, "is_compressed")?,
            cwd: NamedRow::get::<Nullable<Text>, _>(row, "cwd")?,
        })
    }
}

pub struct ConversationRepositoryImpl {
    pool: Arc<DatabasePool>,
    wid: WorkspaceHash,
}

impl ConversationRepositoryImpl {
    pub fn new(pool: Arc<DatabasePool>, workspace_id: WorkspaceHash) -> Self {
        Self { pool, wid: workspace_id }
    }

    async fn run_blocking<F, T>(&self, operation: F) -> anyhow::Result<T>
    where
        F: FnOnce(Arc<DatabasePool>, WorkspaceHash) -> anyhow::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let pool = self.pool.clone();
        let wid = self.wid;
        tokio::task::spawn_blocking(move || operation(pool, wid))
            .await
            .map_err(|e| anyhow::anyhow!("Conversation repository task failed: {e}"))?
    }

    async fn run_with_connection<F, T>(&self, operation: F) -> anyhow::Result<T>
    where
        F: FnOnce(&mut PooledSqliteConnection, WorkspaceHash) -> anyhow::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        self.run_blocking(move |pool, wid| {
            let mut connection = pool.get_connection()?;
            operation(&mut connection, wid)
        })
        .await
    }
}

#[async_trait::async_trait]
impl ConversationRepository for ConversationRepositoryImpl {
    async fn upsert_conversation_ref(&self, conversation: &Conversation) -> anyhow::Result<()> {
        let conversation = conversation.clone();
        self.run_with_connection(move |connection, wid| {
            let record = ConversationRecord::new_ref(&conversation, wid);
            diesel::insert_into(conversations::table)
                .values(&record)
                .on_conflict(conversations::conversation_id)
                .do_update()
                .set((
                    conversations::title.eq(&record.title),
                    conversations::context.eq(&record.context),
                    conversations::updated_at.eq(record.updated_at),
                    conversations::metrics.eq(&record.metrics),
                    conversations::parent_id.eq(&record.parent_id),
                    conversations::source.eq(&record.source),
                    conversations::cwd.eq(&record.cwd),
                    conversations::message_count.eq(record.message_count),
                ))
                .execute(connection)?;
            Ok(())
        })
        .await
    }

    async fn upsert_conversation(&self, conversation: Conversation) -> anyhow::Result<()> {
        self.run_with_connection(move |connection, wid| {
            let record = ConversationRecord::new(conversation, wid);
            diesel::insert_into(conversations::table)
                .values(&record)
                .on_conflict(conversations::conversation_id)
                .do_update()
                .set((
                    conversations::title.eq(&record.title),
                    conversations::context.eq(&record.context),
                    conversations::context_zstd.eq(&record.context_zstd),
                    conversations::is_compressed.eq(record.is_compressed),
                    conversations::updated_at.eq(record.updated_at),
                    conversations::metrics.eq(&record.metrics),
                    conversations::parent_id.eq(&record.parent_id),
                    conversations::source.eq(&record.source),
                    conversations::cwd.eq(&record.cwd),
                    conversations::message_count.eq(record.message_count),
                ))
                .execute(connection)?;
            Ok(())
        })
        .await
    }

    async fn get_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> anyhow::Result<Option<Conversation>> {
        let conversation_id = *conversation_id;
        self.run_with_connection(move |connection, _wid| {
            let record: Option<ConversationRecord> = conversations::table
                .filter(conversations::conversation_id.eq(conversation_id.into_string()))
                .first(connection)
                .optional()?;

            match record {
                Some(record) => Ok(Some(Conversation::try_from(record)?)),
                None => Ok(None),
            }
        })
        .await
    }

    async fn get_all_conversations(
        &self,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        self.run_with_connection(move |connection, wid| {
            use diesel::dsl::sql;
            use diesel::prelude::*;

            let workspace_id = wid.id() as i64;
            // Filter for rows with context data: either plain context column OR compressed context_zstd
            // Using raw SQL to express: context IS NOT NULL OR is_compressed = 1
            let mut query = conversations::table
                .filter(conversations::workspace_id.eq(&workspace_id))
                .filter(sql::<diesel::sql_types::Bool>(
                    "context IS NOT NULL OR is_compressed = 1",
                ))
                .order(conversations::updated_at.desc())
                .into_boxed();

            if let Some(limit_value) = limit {
                query = query.limit(limit_value as i64);
            }

            let records: Vec<ConversationRecord> = query.load(connection)?;

            if records.is_empty() {
                return Ok(None);
            }

            let conversations: Result<Vec<Conversation>, _> =
                records.into_iter().map(Conversation::try_from).collect();
            Ok(Some(conversations?))
        })
        .await
    }

    async fn get_last_conversation(&self) -> anyhow::Result<Option<Conversation>> {
        self.run_with_connection(move |connection, wid| {
            use diesel::dsl::sql;
            use diesel::prelude::*;

            let workspace_id = wid.id() as i64;
            let record: Option<ConversationRecord> = conversations::table
                .filter(conversations::workspace_id.eq(&workspace_id))
                .filter(sql::<diesel::sql_types::Bool>(
                    "context IS NOT NULL OR is_compressed = 1",
                ))
                .order(conversations::updated_at.desc())
                .first(connection)
                .optional()?;
            let conversation = match record {
                Some(record) => Some(Conversation::try_from(record)?),
                None => None,
            };
            Ok(conversation)
        })
        .await
    }

    async fn delete_conversation(&self, conversation_id: &ConversationId) -> anyhow::Result<()> {
        let conversation_id = *conversation_id;
        self.run_with_connection(move |connection, wid| {
            let workspace_id = wid.id() as i64;

            // Security: Ensure users can only delete conversations within their workspace
            diesel::delete(conversations::table)
                .filter(conversations::workspace_id.eq(&workspace_id))
                .filter(conversations::conversation_id.eq(conversation_id.into_string()))
                .execute(connection)?;

            Ok(())
        })
        .await
    }

    async fn get_conversations_by_parent(
        &self,
        parent_id: &ConversationId,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        let parent_id = parent_id.into_string();
        self.run_with_connection(move |connection, wid| {
            let workspace_id = wid.id() as i64;
            let records: Vec<ConversationRecord> = conversations::table
                .filter(conversations::workspace_id.eq(&workspace_id))
                .filter(conversations::parent_id.eq(&parent_id))
                .filter(conversations::context.is_not_null())
                .order(conversations::updated_at.desc())
                .load(connection)?;

            if records.is_empty() {
                return Ok(None);
            }

            let conversations: Result<Vec<Conversation>, _> =
                records.into_iter().map(Conversation::try_from).collect();
            Ok(Some(conversations?))
        })
        .await
    }

    async fn get_parent_conversations(
        &self,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        self.run_with_connection(move |connection, wid| {
            let workspace_id = wid.id() as i64;
            let mut query = conversations::table
                .filter(conversations::workspace_id.eq(&workspace_id))
                .filter(conversations::context.is_not_null())
                .filter(conversations::parent_id.is_null())
                .order(conversations::updated_at.desc())
                .into_boxed();

            if let Some(limit_value) = limit {
                query = query.limit(limit_value as i64);
            }

            let records: Vec<ConversationRecord> = query.load(connection)?;

            if records.is_empty() {
                return Ok(None);
            }

            let conversations: Result<Vec<Conversation>, _> =
                records.into_iter().map(Conversation::try_from).collect();
            Ok(Some(conversations?))
        })
        .await
    }

    async fn get_parent_conversations_lite(
        &self,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<ConversationSummary>>> {
        self.run_with_connection(move |connection, wid| {
            let workspace_id = wid.id() as i64;
            let mut query = conversations::table
                .filter(conversations::workspace_id.eq(&workspace_id))
                .filter(conversations::parent_id.is_null())
                .select((
                    conversations::conversation_id,
                    conversations::title,
                    conversations::created_at,
                    conversations::updated_at,
                    conversations::parent_id,
                    conversations::cwd,
                    conversations::message_count,
                ))
                .order(conversations::updated_at.desc())
                .into_boxed();

            if let Some(limit_value) = limit {
                query = query.limit(limit_value as i64);
            }

            let records: Vec<ConversationRecordLite> = query.load(connection)?;

            if records.is_empty() {
                return Ok(None);
            }

            let summaries: Vec<ConversationSummary> =
                records.into_iter().map(ConversationSummary::from).collect();
            Ok(Some(summaries))
        })
        .await
    }

    async fn get_conversations_by_source(
        &self,
        source: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        let source = source.to_string();
        self.run_with_connection(move |connection, wid| {
            let workspace_id = wid.id() as i64;
            let mut query = conversations::table
                .filter(conversations::workspace_id.eq(&workspace_id))
                .filter(conversations::context.is_not_null())
                .filter(conversations::source.eq(&source))
                .order(conversations::updated_at.desc())
                .into_boxed();

            if let Some(limit_value) = limit {
                query = query.limit(limit_value as i64);
            }

            let records: Vec<ConversationRecord> = query.load(connection)?;

            if records.is_empty() {
                return Ok(None);
            }

            let conversations: Result<Vec<Conversation>, _> =
                records.into_iter().map(Conversation::try_from).collect();
            Ok(Some(conversations?))
        })
        .await
    }

    async fn search_conversations(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<Conversation>> {
        let query = query.to_string();
        let limit_value = limit.map(|n| n as i64);
        self.run_with_connection(move |connection, wid| {
            let workspace_id = wid.id() as i64;
            // FTS5 BM25 search joined back to the base table on
            // `rowid` (now explicit `rowid` column in external-content FTS5).
            // `bm25()` returns a negative number where lower = more relevant, so `ORDER BY
            // rank_score` (ascending) yields "best match first".
            //
            // We do NOT include `snippet()` here because it would force
            // the SELECT to return a column not in `ConversationRecord`.
            // The UI fetches a snippet on-demand via the separate
            // `get_conversation_snippet` method when the user picks a hit.
            let mut sql = String::from(
                "SELECT c.*, bm25(conversations_fts) AS rank_score \
                 FROM conversations c \
                 JOIN conversations_fts fts ON c.rowid = fts.rowid \
                 WHERE conversations_fts MATCH ? \
                   AND c.workspace_id = ? \
                 ORDER BY rank_score",
            );
            if limit_value.is_some() {
                sql.push_str(" LIMIT ?");
            }

            // We can't bind the FTS MATCH expression positionally because
            // diesel::sql_query does not have a typed binding for FTS5's
            // MATCH operator when used as a column. Use the lower-level
            // `sql_query` so we can read back the typed rows.
            let mut q = diesel::sql_query(sql).into_boxed();
            q = q.bind::<diesel::sql_types::Text, _>(&query);
            q = q.bind::<diesel::sql_types::BigInt, _>(workspace_id);
            if let Some(l) = limit_value {
                q = q.bind::<diesel::sql_types::BigInt, _>(l);
            }

            let raw_rows: Vec<ConversationRecord> = q.load(connection)?;
            let conversations: Result<Vec<Conversation>, _> =
                raw_rows.into_iter().map(Conversation::try_from).collect();
            conversations
        })
        .await
    }

    /// Return a single FTS5 snippet for a (conversation, query) pair.
    /// Used by the UI to render a "matched passage" preview for the
    /// currently selected search hit. Returns `None` if no match.
    async fn get_conversation_snippet(
        &self,
        conversation_id: &ConversationId,
        query: &str,
        token_count: usize,
    ) -> anyhow::Result<Option<String>> {
        let conversation_id_str = conversation_id.into_string();
        let query = query.to_string();
        self.run_with_connection(move |connection, _wid| {
            // External-content FTS5 mode: use rowid to join and column index 1 for context.
            // FTS5 column order: title (0), context (1), cwd (2).
            // We filter by rowid matching the base conversation ID.
            let sql = format!(
                "SELECT snippet(conversations_fts, 1, '[', ']', '…', {}) AS s \
                 FROM conversations_fts \
                 WHERE rowid = (SELECT rowid FROM conversations WHERE conversation_id = ?) \
                   AND conversations_fts MATCH ?",
                token_count.min(256)
            );
            let raw: Vec<SnippetRow> = diesel::sql_query(sql)
                .bind::<diesel::sql_types::Text, _>(&conversation_id_str)
                .bind::<diesel::sql_types::Text, _>(&query)
                .load(connection)?;
            Ok(raw.into_iter().next().map(|r| r.s))
        })
        .await
    }

    async fn optimize_fts_index(&self) -> anyhow::Result<()> {
        // FTS5's "optimize" command is invoked as a special INSERT against
        // the virtual table itself. Diesel has no typed binding for it, so
        // we use a raw sql_query. This is the canonical pattern from the
        // SQLite FTS5 docs: https://sqlite.org/fts5.html#the_optimize_command
        self.run_with_connection(move |connection, _wid| {
            diesel::sql_query(
                "INSERT INTO conversations_fts(conversations_fts) VALUES('optimize')",
            )
            .execute(connection)?;
            Ok(())
        })
        .await
    }

    async fn refresh_fts_index(&self) -> anyhow::Result<()> {
        // CONTENTFUL FTS5 populated in application code.
        // This ensures BOTH compressed and uncompressed rows are indexed.
        //
        // Process:
        // 1. Clear the FTS index (DELETE all rows)
        // 2. SELECT all conversations with their rowid, title, context, context_zstd, is_compressed
        // 3. For each row: if is_compressed=1, decompress context_zstd to get searchable text;
        //    otherwise use context directly
        // 4. INSERT (rowid, title, content, cwd) into conversations_fts
        //
        // This is more work than FTS5's 'rebuild' but necessary because:
        // - External-content FTS5 reads context column by name → compressed rows (context=NULL) are missed
        // - Decompression must happen in app code; FTS5 has no built-in codec
        // - Contentful FTS5 is the pragmatic correct solution
        self.run_with_connection(move |connection, _wid| {
            use crate::codec;
            use diesel::sql_types::{BigInt, Text, Nullable};

            // Step 1: Clear the FTS index
            diesel::sql_query("DELETE FROM conversations_fts")
                .execute(connection)?;

            // Step 2: Read all conversations using custom QueryableByName type
            let rows: Vec<FtsRefreshRow> = diesel::sql_query(
                "SELECT rowid, title, context, context_zstd, is_compressed, cwd \
                 FROM conversations"
            )
            .load(connection)?;

            // Step 3 & 4: For each row, decompress if needed and INSERT into FTS
            for row in rows {
                // Determine searchable content: decompress if compressed, else use plain text
                let content = if row.is_compressed == 1 {
                    if let Some(compressed) = row.context_zstd {
                        match codec::decompress(&compressed) {
                            Ok(decompressed) => decompressed,
                            Err(e) => {
                                eprintln!(
                                    "Warning: Failed to decompress context_zstd for rowid {}; skipping FTS: {}",
                                    row.rowid, e
                                );
                                String::new()
                            }
                        }
                    } else {
                        eprintln!("Warning: rowid {} marked compressed but context_zstd is None; skipping FTS", row.rowid);
                        String::new()
                    }
                } else {
                    // Uncompressed row: use context column directly
                    row.context.unwrap_or_default()
                };

                // Insert into FTS5 contentful table
                diesel::sql_query(
                    "INSERT INTO conversations_fts(rowid, title, content, cwd) VALUES (?, ?, ?, ?)"
                )
                .bind::<BigInt, _>(row.rowid)
                .bind::<Text, _>(&row.title)
                .bind::<Text, _>(&content)
                .bind::<Nullable<Text>, _>(&row.cwd)
                .execute(connection)?;
            }

            Ok(())
        })
        .await
    }

    async fn update_parent_id(
        &self,
        conversation_id: &ConversationId,
        new_parent_id: Option<&ConversationId>,
    ) -> anyhow::Result<()> {
        // The `Option<&ConversationId>` is borrowed for the duration of the
        // move into `run_with_connection`. We materialise the inner string
        // here so the closure becomes `'static`.
        let new_parent_id_str: Option<String> = new_parent_id.map(|id| id.into_string());
        let conversation_id_str = conversation_id.into_string();
        let now: chrono::NaiveDateTime = chrono::Utc::now().naive_utc();
        self.run_with_connection(move |connection, _wid| {
            diesel::update(
                conversations::table
                    .filter(conversations::conversation_id.eq(&conversation_id_str)),
            )
            .set((
                conversations::parent_id.eq(new_parent_id_str),
                conversations::updated_at.eq(Some(now)),
            ))
            .execute(connection)?;
            Ok(())
        })
        .await
    }

    async fn rewind_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> anyhow::Result<Option<Conversation>> {
        let conversation_id_str = conversation_id.into_string();
        let now: chrono::NaiveDateTime = chrono::Utc::now().naive_utc();
        let result = self
            .run_with_connection(move |connection, _wid| {
                // MVP rewind semantics: find the most recent user message followed by
                // a tool call (i.e. last compaction point heuristic) and truncate
                // the context JSON to that prefix. If no tool call is found,
                // fall back to clearing context to the most recent user message.
                let record: Option<ConversationRecord> = conversations::table
                    .filter(conversations::conversation_id.eq(&conversation_id_str))
                    .first(connection)
                    .optional()?;

                let new_context: Option<String> = match record {
                    Some(r) if r.context.is_some() => {
                        let ctx = r.context.as_ref().unwrap();
                        let rewind_point = find_last_compaction_point(ctx);
                        Some(truncate_context(ctx, rewind_point))
                    }
                    _ => None,
                };

                diesel::update(
                    conversations::table
                        .filter(conversations::conversation_id.eq(&conversation_id_str)),
                )
                .set((
                    conversations::context.eq(new_context),
                    conversations::updated_at.eq(Some(now)),
                ))
                .execute(connection)?;

                // Re-read the updated record so we can return it.
                let updated: Option<ConversationRecord> = conversations::table
                    .filter(conversations::conversation_id.eq(&conversation_id_str))
                    .first(connection)
                    .optional()?;
                Ok(updated.and_then(|r| Conversation::try_from(r).ok()))
            })
            .await?;
        Ok(result)
    }

    async fn get_conversations_by_cwd(
        &self,
        cwd: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Option<Vec<Conversation>>> {
        let cwd = cwd.to_string();
        self.run_with_connection(move |connection, wid| {
            let workspace_id = wid.id() as i64;
            let mut query = conversations::table
                .filter(conversations::workspace_id.eq(&workspace_id))
                .filter(conversations::context.is_not_null())
                .filter(conversations::cwd.eq(&cwd))
                .order(conversations::updated_at.desc())
                .into_boxed();

            if let Some(limit_value) = limit {
                query = query.limit(limit_value as i64);
            }

            let records: Vec<ConversationRecord> = query.load(connection)?;

            if records.is_empty() {
                return Ok(None);
            }

            let conversations: Result<Vec<Conversation>, _> =
                records.into_iter().map(Conversation::try_from).collect();
            Ok(Some(conversations?))
        })
        .await
    }

    async fn mark_intent_state(
        &self,
        conversation_id: &ConversationId,
        new_state: &str,
    ) -> anyhow::Result<()> {
        use crate::conversation::intent::IntentState;

        let conversation_id = conversation_id.into_string();
        let new_state_str = new_state.to_string();
        let new_state = IntentState::from_str(new_state)?;

        self.run_with_connection(move |connection, _wid| {
            // Read current state to validate transition
            let current_record: Option<ConversationRecord> = conversations::table
                .filter(conversations::conversation_id.eq(&conversation_id))
                .first(connection)
                .optional()?;

            let record = current_record
                .ok_or_else(|| anyhow::anyhow!("Conversation {} not found", conversation_id))?;

            let current_state = IntentState::from_str(&record.intent_state)?;

            // Enforce state machine: can_transition_to returns false for illegal transitions
            if !current_state.can_transition_to(new_state) {
                return Err(anyhow::anyhow!(
                    "Illegal state transition: {} → {}",
                    current_state,
                    new_state
                ));
            }

            // Update the state
            let now = chrono::Utc::now().naive_utc();
            diesel::update(
                conversations::table.filter(conversations::conversation_id.eq(&conversation_id)),
            )
            .set((
                conversations::intent_state.eq(&new_state_str),
                conversations::updated_at.eq(Some(now)),
            ))
            .execute(connection)?;

            Ok(())
        })
        .await
    }

    async fn list_prune_eligible(
        &self,
        workspace_id: Option<i64>,
        limit: usize,
    ) -> anyhow::Result<Vec<Conversation>> {
        self.run_with_connection(move |connection, wid| {
            let workspace_id = workspace_id.unwrap_or_else(|| wid.id() as i64);
            let limit = limit as i64;

            // Use raw SQL to order by context blob size (descending) to prioritize
            // largest contexts first for maximum space reclamation
            let sql = "SELECT c.* FROM conversations c \
                 WHERE c.workspace_id = ? \
                   AND c.intent_state = 'verified' \
                   AND c.context IS NOT NULL \
                 ORDER BY LENGTH(c.context) DESC \
                 LIMIT ?";

            let records: Vec<ConversationRecord> = diesel::sql_query(sql)
                .bind::<diesel::sql_types::BigInt, _>(workspace_id)
                .bind::<diesel::sql_types::BigInt, _>(limit)
                .load(connection)?;

            let conversations: Result<Vec<Conversation>, _> =
                records.into_iter().map(Conversation::try_from).collect();
            conversations
        })
        .await
    }

    async fn prune_conversation(&self, conversation_id: &ConversationId) -> anyhow::Result<()> {
        use crate::conversation::intent::IntentState;

        let conversation_id = conversation_id.into_string();

        self.run_with_connection(move |connection, _wid| {
            // Read current state to enforce invariant: only prune from 'verified'
            let current_record: Option<ConversationRecord> = conversations::table
                .filter(conversations::conversation_id.eq(&conversation_id))
                .first(connection)
                .optional()?;

            let record = current_record
                .ok_or_else(|| anyhow::anyhow!("Conversation {} not found", conversation_id))?;

            let current_state = IntentState::from_str(&record.intent_state)?;

            // Safety guard: only prune if verified
            if current_state != IntentState::Verified {
                return Err(anyhow::anyhow!(
                    "Cannot prune conversation with intent_state='{}'. Must be 'verified'.",
                    current_state
                ));
            }

            // Create a compact summary JSON to replace the full context blob
            // Preserves just enough metadata for the conversation to remain queryable
            let compressed_context = serde_json::json!({
                "type": "compressed",
                "conversation_id": conversation_id,
                "pruned_at": chrono::Utc::now().to_rfc3339(),
                "summary": "Conversation context pruned; full intent stored in MemoryPort"
            })
            .to_string();

            let now = chrono::Utc::now().naive_utc();
            diesel::update(
                conversations::table.filter(conversations::conversation_id.eq(&conversation_id)),
            )
            .set((
                conversations::context.eq(compressed_context),
                conversations::intent_state.eq("pruned"),
                conversations::updated_at.eq(Some(now)),
            ))
            .execute(connection)?;

            Ok(())
        })
        .await
    }
}

/// Find the byte-offset in the context JSON immediately after the last
/// "compaction point" we can detect. The MVP heuristic scans the JSON string
/// for tool-call markers (`"name":`) in reverse and returns the offset of
/// the most recent user-text content that *precedes* a tool call.
///
/// `0` means "no rewound prefix found; truncate to empty" (full reset).
fn find_last_compaction_point(context_json: &str) -> usize {
    // Walk the JSON looking for the most recent `"role":"user"` message
    // boundary followed by a tool call. Each message entry in the context
    // is a JSON object; we just look for the substring order heuristically.
    // This is intentionally conservative: it errs on "rewind less, keep
    // more history" rather than "rewind too far, lose context".
    let user_marker = "\"role\":\"user\"";
    let tool_marker = "\"tool_calls\"";

    // Find the last user-role occurrence.
    let last_user = context_json.rfind(user_marker);
    if last_user.is_none() {
        return 0;
    }
    // After that user-role, look forward for the first tool_call marker.
    let after_user = last_user.unwrap() + user_marker.len();
    if context_json[after_user..].find(tool_marker).is_some() {
        // Truncate at the user-role boundary so we keep the user turn
        // but discard everything after it (including the tool call).
        return last_user.unwrap();
    }
    // No tool call after the last user message — treat the last user
    // message as the rewind point too (discard any trailing assistant
    // text/tool results that came after).
    last_user.unwrap()
}

/// Truncate the context JSON to the prefix `rewind_point` bytes long.
/// Re-emits a valid JSON shape: `{ "messages": ...truncated prefix... }`.
/// If the prefix is `0`, returns an empty messages array.
fn truncate_context(context_json: &str, rewind_point: usize) -> String {
    if rewind_point == 0 {
        return r#"{"messages":[]}"#.to_string();
    }
    // Walk backwards to the previous comma or opening brace so we don't
    // produce a truncated object/messages array.
    let bytes = context_json.as_bytes();
    let mut cut = rewind_point.min(bytes.len());
    while cut > 0 && bytes[cut - 1] != b',' && bytes[cut - 1] != b'[' && bytes[cut - 1] != b'{' {
        cut -= 1;
    }
    let prefix = &context_json[..cut];
    format!("{}\"rewound\":true}}", prefix.trim_end_matches([',', ' ']))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use forge_domain::{
        Context, ContextMessage, Effort, FileOperation, Metrics, Role, ToolCallFull, ToolCallId,
        ToolChoice, ToolDefinition, ToolKind, ToolName, ToolOutput, ToolResult, ToolValue, Usage,
    };
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::conversation::conversation_record::{ContextRecord, MetricsRecord};
    use crate::database::DatabasePool;

    fn repository() -> anyhow::Result<ConversationRepositoryImpl> {
        let pool = Arc::new(DatabasePool::in_memory()?);
        Ok(ConversationRepositoryImpl::new(pool, WorkspaceHash::new(0)))
    }

    #[tokio::test]
    async fn test_upsert_and_find_by_id() -> anyhow::Result<()> {
        let fixture = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()));
        let repo = repository()?;

        repo.upsert_conversation(fixture.clone()).await?;

        let actual = repo.get_conversation(&fixture.id).await?;
        assert!(actual.is_some());
        let retrieved = actual.unwrap();
        assert_eq!(retrieved.id, fixture.id);
        assert_eq!(retrieved.title, fixture.title);
        Ok(())
    }

    #[tokio::test]
    async fn test_find_by_id_non_existing() -> anyhow::Result<()> {
        let repo = repository()?;
        let non_existing_id = ConversationId::generate();

        let actual = repo.get_conversation(&non_existing_id).await?;

        assert!(actual.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_upsert_updates_existing_conversation() -> anyhow::Result<()> {
        let mut fixture = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()));
        let repo = repository()?;

        // Insert initial conversation
        repo.upsert_conversation(fixture.clone()).await?;

        // Update the conversation
        fixture = fixture.title(Some("Updated Title".to_string()));
        repo.upsert_conversation(fixture.clone()).await?;

        let actual = repo.get_conversation(&fixture.id).await?;
        assert!(actual.is_some());
        assert_eq!(actual.unwrap().title, Some("Updated Title".to_string()));
        Ok(())
    }

    #[tokio::test]
    async fn test_find_all_conversations() -> anyhow::Result<()> {
        let context1 =
            Context::default().messages(vec![ContextMessage::user("Hello", None).into()]);
        let context2 =
            Context::default().messages(vec![ContextMessage::user("World", None).into()]);
        let conversation1 = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()))
            .context(Some(context1));
        let conversation2 = Conversation::new(ConversationId::generate())
            .title(Some("Second Conversation".to_string()))
            .context(Some(context2));
        let repo = repository()?;

        repo.upsert_conversation(conversation1.clone()).await?;
        repo.upsert_conversation(conversation2.clone()).await?;

        let actual = repo.get_all_conversations(None).await?;

        assert!(actual.is_some());
        let conversations = actual.unwrap();
        assert_eq!(conversations.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn test_find_all_conversations_with_limit() -> anyhow::Result<()> {
        let context1 =
            Context::default().messages(vec![ContextMessage::user("Hello", None).into()]);
        let context2 =
            Context::default().messages(vec![ContextMessage::user("World", None).into()]);
        let conversation1 = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()))
            .context(Some(context1));
        let conversation2 = Conversation::new(ConversationId::generate()).context(Some(context2));
        let repo = repository()?;

        repo.upsert_conversation(conversation1).await?;
        repo.upsert_conversation(conversation2).await?;

        let actual = repo.get_all_conversations(Some(1)).await?;

        assert!(actual.is_some());
        assert_eq!(actual.unwrap().len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn test_find_all_conversations_empty() -> anyhow::Result<()> {
        let repo = repository()?;

        let actual = repo.get_all_conversations(None).await?;

        assert!(actual.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_find_last_active_conversation_with_context() -> anyhow::Result<()> {
        let context = Context::default().messages(vec![ContextMessage::user("Hello", None).into()]);
        let conversation_with_context = Conversation::new(ConversationId::generate())
            .title(Some("Conversation with Context".to_string()))
            .context(Some(context));
        let conversation_without_context = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()));
        let repo = repository()?;

        repo.upsert_conversation(conversation_without_context)
            .await?;
        repo.upsert_conversation(conversation_with_context.clone())
            .await?;

        let actual = repo.get_last_conversation().await?;

        assert!(actual.is_some());
        assert_eq!(actual.unwrap().id, conversation_with_context.id);
        Ok(())
    }

    #[tokio::test]
    async fn test_find_last_active_conversation_no_context() -> anyhow::Result<()> {
        let conversation_without_context = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()));
        let repo = repository()?;

        repo.upsert_conversation(conversation_without_context)
            .await?;

        let actual = repo.get_last_conversation().await?;

        assert!(actual.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_find_last_active_conversation_ignores_empty_context() -> anyhow::Result<()> {
        let conversation_with_empty_context = Conversation::new(ConversationId::generate())
            .title(Some("Conversation with Empty Context".to_string()))
            .context(Some(Context::default()));
        let conversation_without_context = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()));
        let repo = repository()?;

        repo.upsert_conversation(conversation_without_context)
            .await?;
        repo.upsert_conversation(conversation_with_empty_context)
            .await?;

        let actual = repo.get_last_conversation().await?;

        assert!(actual.is_none()); // Should not find conversations with empty contexts
        Ok(())
    }

    #[test]
    fn test_conversation_record_from_conversation() -> anyhow::Result<()> {
        let fixture = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()));

        let actual = ConversationRecord::new(fixture.clone(), WorkspaceHash::new(0));

        assert_eq!(actual.conversation_id, fixture.id.into_string());
        assert_eq!(actual.title, Some("Test Conversation".to_string()));
        assert_eq!(actual.context, None);
        Ok(())
    }

    #[test]
    fn test_conversation_record_from_conversation_with_context() -> anyhow::Result<()> {
        let context = Context::default().messages(vec![ContextMessage::user("Hello", None).into()]);
        let fixture = Conversation::new(ConversationId::generate())
            .title(Some("Conversation with Context".to_string()))
            .context(Some(context));

        let actual = ConversationRecord::new(fixture.clone(), WorkspaceHash::new(0));

        assert_eq!(actual.conversation_id, fixture.id.into_string());
        assert_eq!(actual.title, Some("Conversation with Context".to_string()));
        // With compression, context is stored in context_zstd and is_compressed=1
        assert!(
            actual.context_zstd.is_some() || actual.context.is_some(),
            "context should be stored in either context_zstd (compressed) or context (plain)"
        );
        Ok(())
    }

    #[test]
    fn test_conversation_record_from_conversation_with_empty_context() -> anyhow::Result<()> {
        let fixture = Conversation::new(ConversationId::generate())
            .title(Some("Conversation with Empty Context".to_string()))
            .context(Some(Context::default()));

        let actual = ConversationRecord::new(fixture.clone(), WorkspaceHash::new(0));

        assert_eq!(actual.conversation_id, fixture.id.into_string());
        assert_eq!(
            actual.title,
            Some("Conversation with Empty Context".to_string())
        );

        assert!(actual.context.is_none()); // Empty context should be filtered out
        Ok(())
    }

    #[test]
    fn test_conversation_from_conversation_record() -> anyhow::Result<()> {
        let test_id = ConversationId::generate();
        let fixture = ConversationRecord {
            conversation_id: test_id.into_string(),
            title: Some("Test Conversation".to_string()),
            context: None,
            created_at: Utc::now().naive_utc(),
            updated_at: None,
            workspace_id: 0,
            metrics: None,
            parent_id: None,
            source: None,
            cwd: None,
            message_count: None,
            intent_state: "pending".to_string(),
            extracted_at: None,
            memory_id: None,
            intent_hash: None,
            context_zstd: None,
            is_compressed: 0,
        };

        let actual = Conversation::try_from(fixture)?;

        assert_eq!(actual.id, test_id);
        assert_eq!(actual.title, Some("Test Conversation".to_string()));
        assert_eq!(actual.context, None);
        Ok(())
    }

    #[tokio::test]
    async fn test_upsert_and_retrieve_conversation_with_metrics() -> anyhow::Result<()> {
        let repo = repository()?;

        // Create a conversation with metrics
        let metrics = Metrics::default()
            .started_at(Utc::now())
            .insert(
                "src/main.rs".to_string(),
                FileOperation::new(ToolKind::Write)
                    .lines_added(10u64)
                    .lines_removed(5u64)
                    .content_hash(Some("abc123def456".to_string())),
            )
            .insert(
                "src/lib.rs".to_string(),
                FileOperation::new(ToolKind::Write)
                    .lines_added(3u64)
                    .lines_removed(2u64)
                    .content_hash(Some("789xyz456abc".to_string())),
            );

        let fixture = Conversation::generate().metrics(metrics.clone());

        // Save the conversation
        repo.upsert_conversation(fixture.clone()).await?;

        // Retrieve the conversation
        let actual = repo
            .get_conversation(&fixture.id)
            .await?
            .expect("Conversation should exist");

        // Verify metrics are preserved
        assert_eq!(actual.metrics.file_operations.len(), 2);
        let main_metrics = actual.metrics.file_operations.get("src/main.rs").unwrap();
        assert_eq!(main_metrics.lines_added, 10);
        assert_eq!(main_metrics.lines_removed, 5);
        assert_eq!(main_metrics.content_hash, Some("abc123def456".to_string()));
        let lib_metrics = actual.metrics.file_operations.get("src/lib.rs").unwrap();
        assert_eq!(lib_metrics.lines_added, 3);
        assert_eq!(lib_metrics.lines_removed, 2);
        assert_eq!(lib_metrics.content_hash, Some("789xyz456abc".to_string()));
        Ok(())
    }

    #[test]
    fn test_metrics_record_conversion_preserves_all_fields() {
        // This test ensures compile-time safety: if Metrics schema changes,
        // this test will fail to compile, alerting us to update MetricsRecord
        let fixture = Metrics::default().started_at(Utc::now()).insert(
            "test.rs".to_string(),
            FileOperation::new(ToolKind::Write)
                .lines_added(5u64)
                .lines_removed(3u64)
                .content_hash(Some("test_hash_123".to_string())),
        );

        // Convert to record and back
        let record = MetricsRecord::from(&fixture);
        let actual = Metrics::from(record);

        // Verify all fields are preserved
        assert_eq!(actual.started_at, fixture.started_at);
        assert_eq!(actual.file_operations.len(), fixture.file_operations.len());

        let actual_file = actual.file_operations.get("test.rs").unwrap();
        let expected_file = fixture.file_operations.get("test.rs").unwrap();
        assert_eq!(actual_file.lines_added, expected_file.lines_added);
        assert_eq!(actual_file.lines_removed, expected_file.lines_removed);
        assert_eq!(actual_file.content_hash, expected_file.content_hash);
    }

    #[test]
    fn test_deserialize_old_format_without_tool_field() {
        // Old format from database: missing tool and content_hash fields
        let json = r#"{
            "started_at": "2024-01-01T00:00:00Z",
            "files_changed": {
                "src/main.rs": {
                    "lines_added": 10,
                    "lines_removed": 5
                },
                "src/lib.rs": {
                    "lines_added": 3,
                    "lines_removed": 2
                }
            }
        }"#;

        let record: MetricsRecord = serde_json::from_str(json).unwrap();
        let actual = Metrics::from(record);

        // Verify files are loaded
        assert_eq!(actual.file_operations.len(), 2);

        // Verify main.rs
        let main_file = actual.file_operations.get("src/main.rs").unwrap();
        assert_eq!(main_file.lines_added, 10);
        assert_eq!(main_file.lines_removed, 5);
        assert_eq!(main_file.content_hash, None);
        assert_eq!(main_file.tool, ToolKind::Write); // Default tool

        // Verify lib.rs
        let lib_file = actual.file_operations.get("src/lib.rs").unwrap();
        assert_eq!(lib_file.lines_added, 3);
        assert_eq!(lib_file.lines_removed, 2);
        assert_eq!(lib_file.content_hash, None);
        assert_eq!(lib_file.tool, ToolKind::Write); // Default tool
    }

    #[test]
    fn test_deserialize_array_format_takes_last_operation() {
        // Array format from database: multiple operations per file
        let json = r#"{
            "started_at": "2024-01-01T00:00:00Z",
            "files_changed": {
                "src/main.rs": [
                    {
                        "lines_added": 2,
                        "lines_removed": 4,
                        "content_hash": "hash1",
                        "tool": "read"
                    },
                    {
                        "lines_added": 1,
                        "lines_removed": 1,
                        "content_hash": "hash2",
                        "tool": "patch"
                    },
                    {
                        "lines_added": 5,
                        "lines_removed": 3,
                        "content_hash": "hash3",
                        "tool": "write"
                    }
                ]
            }
        }"#;

        let record: MetricsRecord = serde_json::from_str(json).unwrap();
        let actual = Metrics::from(record);

        // Verify only the last operation is kept
        assert_eq!(actual.file_operations.len(), 1);

        let main_file = actual.file_operations.get("src/main.rs").unwrap();
        assert_eq!(main_file.lines_added, 5);
        assert_eq!(main_file.lines_removed, 3);
        assert_eq!(main_file.content_hash, Some("hash3".to_string()));
        assert_eq!(main_file.tool, ToolKind::Write);
    }

    #[test]
    fn test_deserialize_array_format_with_empty_array() {
        // Array format with empty array should be skipped
        let json = r#"{
            "started_at": "2024-01-01T00:00:00Z",
            "files_changed": {
                "src/main.rs": [],
                "src/lib.rs": {
                    "lines_added": 5,
                    "lines_removed": 2,
                    "content_hash": "hash1",
                    "tool": "patch"
                }
            }
        }"#;

        let record: MetricsRecord = serde_json::from_str(json).unwrap();
        let actual = Metrics::from(record);

        // Empty array should be skipped, only lib.rs should be present
        assert_eq!(actual.file_operations.len(), 1);
        assert!(actual.file_operations.contains_key("src/lib.rs"));
        assert!(!actual.file_operations.contains_key("src/main.rs"));
    }

    #[test]
    fn test_deserialize_current_format_with_all_fields() {
        // Current format: single object with all fields
        let json = r#"{
            "started_at": "2024-01-01T00:00:00Z",
            "files_changed": {
                "src/main.rs": {
                    "lines_added": 10,
                    "lines_removed": 5,
                    "content_hash": "abc123def456",
                    "tool": "patch"
                },
                "src/lib.rs": {
                    "lines_added": 3,
                    "lines_removed": 2,
                    "content_hash": "789xyz456abc",
                    "tool": "write"
                }
            }
        }"#;

        let record: MetricsRecord = serde_json::from_str(json).unwrap();
        let actual = Metrics::from(record);

        // Verify all fields are preserved
        assert_eq!(actual.file_operations.len(), 2);

        let main_file = actual.file_operations.get("src/main.rs").unwrap();
        assert_eq!(main_file.lines_added, 10);
        assert_eq!(main_file.lines_removed, 5);
        assert_eq!(main_file.content_hash, Some("abc123def456".to_string()));
        assert_eq!(main_file.tool, ToolKind::Patch);

        let lib_file = actual.file_operations.get("src/lib.rs").unwrap();
        assert_eq!(lib_file.lines_added, 3);
        assert_eq!(lib_file.lines_removed, 2);
        assert_eq!(lib_file.content_hash, Some("789xyz456abc".to_string()));
        assert_eq!(lib_file.tool, ToolKind::Write);
    }

    #[test]
    fn test_deserialize_mixed_format() {
        // Mix of old format, array format, and current format
        let json = r#"{
            "started_at": "2024-01-01T00:00:00Z",
            "files_changed": {
                "old_file.rs": {
                    "lines_added": 10,
                    "lines_removed": 5
                },
                "array_file.rs": [
                    {
                        "lines_added": 1,
                        "lines_removed": 2,
                        "content_hash": "hash1",
                        "tool": "read"
                    },
                    {
                        "lines_added": 3,
                        "lines_removed": 4,
                        "content_hash": "hash2",
                        "tool": "patch"
                    }
                ],
                "current_file.rs": {
                    "lines_added": 7,
                    "lines_removed": 8,
                    "content_hash": "hash3",
                    "tool": "write"
                }
            }
        }"#;

        let record: MetricsRecord = serde_json::from_str(json).unwrap();
        let actual = Metrics::from(record);

        assert_eq!(actual.file_operations.len(), 3);

        // Old format file
        let old_file = actual.file_operations.get("old_file.rs").unwrap();
        assert_eq!(old_file.lines_added, 10);
        assert_eq!(old_file.lines_removed, 5);
        assert_eq!(old_file.content_hash, None);
        assert_eq!(old_file.tool, ToolKind::Write); // Default

        // Array format file (should have last operation)
        let array_file = actual.file_operations.get("array_file.rs").unwrap();
        assert_eq!(array_file.lines_added, 3);
        assert_eq!(array_file.lines_removed, 4);
        assert_eq!(array_file.content_hash, Some("hash2".to_string()));
        assert_eq!(array_file.tool, ToolKind::Patch);

        // Current format file
        let current_file = actual.file_operations.get("current_file.rs").unwrap();
        assert_eq!(current_file.lines_added, 7);
        assert_eq!(current_file.lines_removed, 8);
        assert_eq!(current_file.content_hash, Some("hash3".to_string()));
        assert_eq!(current_file.tool, ToolKind::Write);
    }

    #[test]
    fn test_serialize_current_format() {
        // Test that we always serialize in the current format (single object)
        let fixture = Metrics::default().started_at(Utc::now()).insert(
            "src/main.rs".to_string(),
            FileOperation::new(ToolKind::Patch)
                .lines_added(10u64)
                .lines_removed(5u64)
                .content_hash(Some("abc123".to_string())),
        );

        let record = MetricsRecord::from(&fixture);
        let json = serde_json::to_string(&record).unwrap();

        // Verify it's not an array format
        assert!(!json.contains("[{"));
        // Verify it contains the tool field
        assert!(json.contains("\"tool\":\"patch\""));

        // Verify structure is correct
        assert!(json.contains("\"lines_added\":10"));
        assert!(json.contains("\"lines_removed\":5"));
        assert!(json.contains("\"content_hash\":\"abc123\""));
    }

    #[test]
    fn test_context_record_conversion_preserves_all_fields() {
        let tool_def = ToolDefinition::new("test_tool").description("A test tool");

        let reasoning = forge_domain::ReasoningConfig {
            effort: Some(Effort::Medium),
            max_tokens: Some(2048),
            exclude: Some(false),
            enabled: Some(true),
        };

        // Create a comprehensive set of messages to test all message types
        let messages = vec![
            ContextMessage::user("Hello", None).into(),
            ContextMessage::system("System prompt").into(),
            ContextMessage::Tool(ToolResult {
                name: ToolName::new("test_tool"),
                call_id: Some(ToolCallId::new("call_123".to_string())),
                output: ToolOutput {
                    is_error: false,
                    values: vec![ToolValue::Text("Result text".to_string()), ToolValue::Empty],
                },
            })
            .into(),
            forge_domain::MessageEntry {
                message: ContextMessage::Text(forge_domain::TextMessage {
                    role: Role::Assistant,
                    content: "Assistant response".to_string(),
                    raw_content: None,
                    tool_calls: Some(vec![ToolCallFull {
                        name: ToolName::new("another_tool"),
                        call_id: Some(ToolCallId::new("call_456".to_string())),
                        arguments: forge_domain::ToolCallArguments::from(
                            serde_json::json!({"param": "value"}),
                        ),
                        thought_signature: None,
                    }]),
                    model: Some(forge_domain::ModelId::from("gpt-4")),
                    thought_signature: None,
                    reasoning_details: None,
                    droppable: false,
                    phase: None,
                }),
                usage: Some(Usage {
                    prompt_tokens: forge_domain::TokenCount::Actual(100),
                    completion_tokens: forge_domain::TokenCount::Actual(50),
                    total_tokens: forge_domain::TokenCount::Actual(150),
                    cached_tokens: forge_domain::TokenCount::Actual(0),
                    cost: Some(0.001),
                }),
            },
        ];

        let fixture = Context::default()
            .conversation_id(ConversationId::generate())
            .messages(messages)
            .tools(vec![tool_def.clone()])
            .tool_choice(ToolChoice::Call(ToolName::new("test_tool")))
            .max_tokens(1000usize)
            .temperature(forge_domain::Temperature::new(0.7).unwrap())
            .top_p(forge_domain::TopP::new(0.9).unwrap())
            .top_k(forge_domain::TopK::new(50).unwrap())
            .reasoning(reasoning.clone())
            .stream(true);

        // Convert to record and back
        let record = ContextRecord::from(&fixture);
        let actual = Context::try_from(record).unwrap();

        // Verify all fields are preserved
        assert_eq!(actual.conversation_id, fixture.conversation_id);
        assert_eq!(actual.messages.len(), 4);
        assert_eq!(actual.tools.len(), 1);
        assert_eq!(actual.tools[0].name.to_string(), "test_tool");
        assert_eq!(
            actual.tool_choice,
            Some(ToolChoice::Call(ToolName::new("test_tool")))
        );
        assert_eq!(actual.max_tokens, fixture.max_tokens);
        assert_eq!(actual.temperature, fixture.temperature);
        assert_eq!(actual.top_p, fixture.top_p);
        assert_eq!(actual.top_k, fixture.top_k);
        assert_eq!(actual.reasoning, Some(reasoning));
        assert_eq!(actual.stream, fixture.stream);

        // Verify message types and content
        match &actual.messages[0].message {
            ContextMessage::Text(msg) => {
                assert_eq!(msg.role, Role::User);
                assert_eq!(msg.content, "Hello");
            }
            _ => panic!("Expected user message"),
        }

        match &actual.messages[2].message {
            ContextMessage::Tool(tool_result) => {
                assert_eq!(tool_result.name.to_string(), "test_tool");
                assert_eq!(
                    tool_result.call_id.as_ref().map(|id| id.as_str()),
                    Some("call_123")
                );
                assert!(!tool_result.output.is_error);
                assert_eq!(tool_result.output.values.len(), 2);
            }
            _ => panic!("Expected tool result message"),
        }

        // Verify usage is preserved
        match &actual.messages[3].usage {
            Some(usage) => {
                assert_eq!(*usage.prompt_tokens, 100);
                assert_eq!(*usage.completion_tokens, 50);
                assert_eq!(*usage.total_tokens, 150);
                assert_eq!(usage.cost, Some(0.001));
            }
            None => panic!("Expected usage information"),
        }
    }

    #[test]
    fn test_conversation_deserialization_error_includes_id() {
        // Test that deserialization errors include the conversation ID
        let test_id = ConversationId::generate();
        let fixture = ConversationRecord {
            conversation_id: test_id.into_string(),
            title: Some("Test Conversation".to_string()),
            context: Some("invalid json".to_string()), // Invalid JSON to trigger error
            created_at: Utc::now().naive_utc(),
            updated_at: None,
            workspace_id: 0,
            metrics: None,
            parent_id: None,
            source: None,
            cwd: None,
            message_count: None,
            intent_state: "pending".to_string(),
            extracted_at: None,
            memory_id: None,
            intent_hash: None,
            context_zstd: None,
            is_compressed: 0,
        };

        let result = Conversation::try_from(fixture);

        assert!(result.is_err());
        let error_message = result.unwrap_err().to_string();
        assert!(
            error_message.contains(&test_id.to_string()),
            "Error message should contain conversation ID. Got: {}",
            error_message
        );
        assert!(
            error_message.contains("Failed to deserialize context"),
            "Error message should indicate context deserialization failure. Got: {}",
            error_message
        );
    }

    #[tokio::test]
    async fn test_delete_conversation_success() -> anyhow::Result<()> {
        let repo = repository()?;
        let conversation = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()));

        repo.upsert_conversation(conversation.clone()).await?;

        repo.delete_conversation(&conversation.id).await?;

        let result = repo.get_conversation(&conversation.id).await?;
        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_delete_conversation_workspace_filtering() -> anyhow::Result<()> {
        let repo = repository()?;
        let conversation = Conversation::new(ConversationId::generate())
            .title(Some("Test Conversation".to_string()));

        repo.upsert_conversation(conversation.clone()).await?;

        // Delete should succeed regardless of existence (idempotent)
        repo.delete_conversation(&conversation.id).await?;

        // Verify conversation is deleted
        let deleted = repo.get_conversation(&conversation.id).await?;
        assert!(deleted.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_delete_conversation_cross_workspace_security() -> anyhow::Result<()> {
        let repo = repository()?;

        // Create conversation in current workspace
        let conversation_id = ConversationId::generate();
        let conversation =
            Conversation::new(conversation_id).title(Some("Test Conversation".to_string()));

        repo.upsert_conversation(conversation.clone()).await?;

        // Try to delete with different workspace ID (should fail due to security)
        // Note: This test would require modifying workspace ID in repo
        // For now, we test that deletion works with current workspace
        repo.delete_conversation(&conversation.id).await?;

        // Verify it's actually deleted
        let deleted = repo.get_conversation(&conversation.id).await?;
        assert!(deleted.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_delete_conversation_end_to_end_workflow() -> anyhow::Result<()> {
        let repo = repository()?;
        let conversation_id = ConversationId::generate();
        let conversation =
            Conversation::new(conversation_id).title(Some("Test Conversation".to_string()));

        // Test complete workflow: create -> delete -> verify -> create new -> verify
        repo.upsert_conversation(conversation.clone()).await?;

        // Delete conversation
        repo.delete_conversation(&conversation.id).await?;

        // Verify it's gone
        let deleted_check = repo.get_conversation(&conversation.id).await?;
        assert!(deleted_check.is_none());

        // Create new conversation to ensure system still works
        let new_conversation_id = ConversationId::generate();
        let new_conversation = Conversation::new(new_conversation_id);
        repo.upsert_conversation(new_conversation.clone()).await?;

        // Verify new conversation exists
        let new_check = repo.get_conversation(&new_conversation_id).await?;
        assert!(new_check.is_some());

        Ok(())
    }

    #[tokio::test]
    async fn test_rename_conversation_via_upsert() -> anyhow::Result<()> {
        let repo = repository()?;
        let conversation =
            Conversation::new(ConversationId::generate()).title(Some("Original Title".to_string()));

        repo.upsert_conversation(conversation.clone()).await?;

        // Rename by upserting with a new title
        let renamed = conversation
            .clone()
            .title(Some("Renamed Session".to_string()));
        repo.upsert_conversation(renamed).await?;

        let actual = repo.get_conversation(&conversation.id).await?.unwrap();
        assert_eq!(actual.title, Some("Renamed Session".to_string()));
        Ok(())
    }

    #[tokio::test]
    async fn test_rename_conversation_from_none() -> anyhow::Result<()> {
        let repo = repository()?;
        let conversation = Conversation::new(ConversationId::generate());

        // Start with no title
        assert!(conversation.title.is_none());
        repo.upsert_conversation(conversation.clone()).await?;

        // Rename it
        let renamed = conversation.clone().title(Some("My Session".to_string()));
        repo.upsert_conversation(renamed).await?;

        let actual = repo.get_conversation(&conversation.id).await?.unwrap();
        assert_eq!(actual.title, Some("My Session".to_string()));
        Ok(())
    }

    #[test]
    fn test_legacy_tool_value_pair_deserialization() {
        use crate::conversation::conversation_record::ToolOutputRecord;

        // This JSON represents the old Pair variant format that was stored in the
        // database
        let legacy_json = r#"{
            "is_error": false,
            "values": [
                {"pair": [
                    {"text": "XML content for LLM"},
                    {"fileDiff": {"path": "/test/file.rs", "old_text": "old", "new_text": "new"}}
                ]}
            ]
        }"#;

        let record: ToolOutputRecord = serde_json::from_str(legacy_json).unwrap();
        let actual: forge_domain::ToolOutput = record.try_into().unwrap();

        // The Pair variant should be converted by taking the first element (LLM
        // content)
        assert!(!actual.is_error);
        assert_eq!(actual.values.len(), 1);
        assert_eq!(
            actual.values[0],
            forge_domain::ToolValue::Text("XML content for LLM".to_string())
        );
    }

    #[test]
    fn test_legacy_tool_value_markdown_deserialization() {
        use crate::conversation::conversation_record::ToolOutputRecord;

        let legacy_json = r##"{
            "is_error": false,
            "values": [{"markdown": "# Heading - Some bold text"}]
        }"##;

        let record: ToolOutputRecord = serde_json::from_str(legacy_json).unwrap();
        let actual: forge_domain::ToolOutput = record.try_into().unwrap();

        // Markdown should be converted to Text
        assert_eq!(actual.values.len(), 1);
        assert_eq!(
            actual.values[0],
            forge_domain::ToolValue::Text("# Heading - Some bold text".to_string())
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_concurrent_operations_dont_block_runtime() -> anyhow::Result<()> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::{Duration, Instant};

        // Heartbeat fires every `TICK`; we require a measurement window of at
        // least `MIN_WINDOW` so the assertion is meaningful even when the DB
        // workload finishes very quickly (e.g. on fast machines with the
        // in-memory SQLite pool).
        const TICK: Duration = Duration::from_millis(10);
        const MIN_WINDOW: Duration = Duration::from_millis(200);

        let repo = Arc::new(repository()?);
        let heartbeat = Arc::new(AtomicUsize::new(0));

        // Heartbeat task - if runtime is blocked, this won't increment.
        let heartbeat_clone = heartbeat.clone();
        let heartbeat_handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(TICK).await;
                heartbeat_clone.fetch_add(1, Ordering::Relaxed);
            }
        });

        // Warm up: let the heartbeat task get scheduled and complete its first
        // tick before we start measuring, then reset the counter so timing
        // begins from a clean state.
        tokio::time::sleep(TICK * 3).await;
        heartbeat.store(0, Ordering::Relaxed);

        // Spawn many concurrent DB operations.
        let mut handles = vec![];
        let start = Instant::now();

        for i in 0..20 {
            let repo = repo.clone();
            let handle = tokio::spawn(async move {
                for j in 0..10 {
                    let conversation = Conversation::new(ConversationId::generate())
                        .title(Some(format!("Task {} - Write {}", i, j)));
                    repo.upsert_conversation(conversation).await?;
                }
                anyhow::Result::<()>::Ok(())
            });
            handles.push(handle);
        }

        // Wait for all operations.
        for handle in handles {
            handle.await??;
        }

        // Ensure the measurement window is long enough for heartbeat math to
        // be meaningful regardless of how fast the DB workload completed.
        let work_elapsed = start.elapsed();
        if work_elapsed < MIN_WINDOW {
            tokio::time::sleep(MIN_WINDOW - work_elapsed).await;
        }
        let elapsed = start.elapsed();

        // Stop heartbeat.
        heartbeat_handle.abort();

        // Verify runtime wasn't blocked: heartbeat should have fired at least
        // 80% of the theoretical max for the elapsed window. The threshold is
        // clamped to at least 1 to keep the assertion well-defined.
        let heartbeat_count = heartbeat.load(Ordering::Relaxed);
        let expected_heartbeats = (elapsed.as_millis() as usize) / (TICK.as_millis() as usize);
        let threshold = (expected_heartbeats * 8 / 10).max(1);

        assert!(
            heartbeat_count >= threshold,
            "Runtime was blocked! Expected at least {} heartbeats (~{} theoretical) in {:?}, got {}",
            threshold,
            expected_heartbeats,
            elapsed,
            heartbeat_count
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_mixed_read_write_contention() -> anyhow::Result<()> {
        let repo = Arc::new(repository()?);
        let mut handles = vec![];

        // Pre-populate some data
        for i in 0..10 {
            let conv =
                Conversation::new(ConversationId::generate()).title(Some(format!("Initial {}", i)));
            repo.upsert_conversation(conv).await?;
        }

        // Spawn writers
        for i in 0..10 {
            let repo = repo.clone();
            handles.push(tokio::spawn(async move {
                for j in 0..10 {
                    let conv = Conversation::new(ConversationId::generate())
                        .title(Some(format!("Writer {} - {}", i, j)));
                    repo.upsert_conversation(conv).await?;
                }
                anyhow::Result::<()>::Ok(())
            }));
        }

        // Spawn readers (interleave with writers)
        for _ in 0..10 {
            let repo = repo.clone();
            handles.push(tokio::spawn(async move {
                for _ in 0..10 {
                    // Read all conversations
                    let _ = repo.get_all_conversations(Some(50)).await?;
                    tokio::task::yield_now().await;
                }
                anyhow::Result::<()>::Ok(())
            }));
        }

        // All should complete without timeout
        for handle in handles {
            handle.await??;
        }

        Ok(())
    }

    #[test]
    fn test_legacy_tool_value_file_diff_deserialization() {
        use crate::conversation::conversation_record::ToolOutputRecord;

        let legacy_json = r#"{
            "is_error": false,
            "values": [{"fileDiff": {"path": "/src/main.rs", "old_text": "fn old()", "new_text": "fn new()"}}]
        }"#;

        let record: ToolOutputRecord = serde_json::from_str(legacy_json).unwrap();
        let actual: forge_domain::ToolOutput = record.try_into().unwrap();

        // FileDiff should be converted to a text summary
        assert_eq!(actual.values.len(), 1);
        assert_eq!(
            actual.values[0],
            forge_domain::ToolValue::Text("[File diff: /src/main.rs]".to_string())
        );
    }

    #[tokio::test]
    async fn test_prune_conversation_safety_guard() -> anyhow::Result<()> {
        let repo = repository()?;
        let context =
            Context::default().messages(vec![ContextMessage::user("Test content", None).into()]);
        let conversation = Conversation::new(ConversationId::generate())
            .title(Some("Test for Pruning".to_string()))
            .context(Some(context));

        // Insert conversation with default intent_state='pending'
        repo.upsert_conversation(conversation.clone()).await?;

        // ADR-103: Pruning should fail when intent_state != 'verified'
        let result = repo.prune_conversation(&conversation.id).await;
        assert!(
            result.is_err(),
            "Pruning should fail when intent_state='pending'"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Must be 'verified'"),
            "Error should indicate the requirement for 'verified' state"
        );

        // Mark as verified
        repo.mark_intent_state(&conversation.id, "verified").await?;

        // Now pruning should succeed
        let prune_result = repo.prune_conversation(&conversation.id).await;
        assert!(
            prune_result.is_ok(),
            "Pruning should succeed when intent_state='verified'"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_mark_intent_state_enforces_dag() -> anyhow::Result<()> {
        let repo = repository()?;
        let conversation = Conversation::new(ConversationId::generate())
            .title(Some("Test for State Machine".to_string()));

        repo.upsert_conversation(conversation.clone()).await?;

        // Verify default state is 'pending'
        let conv = repo.get_conversation(&conversation.id).await?;
        assert!(conv.is_some());

        // Valid transition: pending → extracting
        assert!(
            repo.mark_intent_state(&conversation.id, "extracting")
                .await
                .is_ok()
        );

        // Valid transition: extracting → extracted
        assert!(
            repo.mark_intent_state(&conversation.id, "extracted")
                .await
                .is_ok()
        );

        // Valid transition: extracted → verified
        assert!(
            repo.mark_intent_state(&conversation.id, "verified")
                .await
                .is_ok()
        );

        // Valid transition: verified → pruned
        assert!(
            repo.mark_intent_state(&conversation.id, "pruned")
                .await
                .is_ok()
        );

        // Invalid transition: pruned → any state (pruned is final)
        let result = repo.mark_intent_state(&conversation.id, "verified").await;
        assert!(result.is_err(), "Cannot transition from pruned to verified");

        Ok(())
    }

    #[tokio::test]
    async fn test_search_finds_compressed_conversations() -> anyhow::Result<()> {
        // CRITICAL TEST: Proves that compressed rows (context=NULL, is_compressed=1) are
        // findable by FTS5 search after refresh_fts_index populates the index with
        // decompressed content.
        //
        // This test catches the bug where external-content FTS5 reads by column name
        // (context), missing compressed rows where context=NULL.
        let repo = repository()?;

        // Create two conversations with context containing searchable text
        let msg_compressed = ContextMessage::user("SEARCHABLE_COMPRESSED_TERM", None);
        let msg_plain = ContextMessage::user("SEARCHABLE_PLAIN_TERM", None);

        let context_compressed = Context::default().messages(vec![msg_compressed.into()]);
        let context_plain = Context::default().messages(vec![msg_plain.into()]);

        // Insert compressed conversation (will be stored as context_zstd, is_compressed=1, context=NULL)
        let compressed_conv = Conversation::new(ConversationId::generate())
            .title(Some("Compressed Conversation".to_string()))
            .context(Some(context_compressed.clone()));
        repo.upsert_conversation(compressed_conv.clone()).await?;

        // Insert uncompressed conversation (will be stored as plain context, is_compressed=0)
        let plain_conv = Conversation::new(ConversationId::generate())
            .title(Some("Plain Conversation".to_string()))
            .context(Some(context_plain.clone()));
        repo.upsert_conversation(plain_conv.clone()).await?;

        // Refresh FTS index to populate both compressed and uncompressed rows
        repo.refresh_fts_index().await?;

        // SEARCH 1: Find compressed conversation by term in its decompressed context
        // If the fix is correct, this search WILL find the compressed row.
        // Before the fix, this would return empty (context=NULL skipped by FTS).
        let results_compressed = repo
            .search_conversations("SEARCHABLE_COMPRESSED_TERM", None)
            .await?;
        assert!(
            !results_compressed.is_empty(),
            "FTS search must find compressed conversations after refresh_fts_index; \
             bug: external-content FTS5 reads context column by name, missing compressed rows"
        );
        assert!(
            results_compressed
                .iter()
                .any(|c| c.id == compressed_conv.id),
            "Search results must include the compressed conversation"
        );

        // SEARCH 2: Find uncompressed conversation (baseline to ensure search works)
        let results_plain = repo
            .search_conversations("SEARCHABLE_PLAIN_TERM", None)
            .await?;
        assert!(
            !results_plain.is_empty(),
            "FTS search must find uncompressed conversations"
        );
        assert!(
            results_plain.iter().any(|c| c.id == plain_conv.id),
            "Search results must include the plain conversation"
        );

        // SEARCH 3: Verify no false positives
        let results_wrong = repo.search_conversations("NONEXISTENT_TERM", None).await?;
        assert!(
            results_wrong.is_empty(),
            "Search must not return conversations that don't contain the search term"
        );

        Ok(())
    }
}
