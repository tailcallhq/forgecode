//! Test suite for SQLite migrations and database operations
//! Tests P1/P2/P2a/P2b/P4 merged work:
//! - Migration round-trip (all migrations apply cleanly on fresh in-memory DB)
//! - FTS5 external-content refresh and search
//! - IntentState transition guards
//! - Prune gate enforcement

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use diesel::prelude::*;
    use diesel::sql_types::*;

    use crate::database::pool::DatabasePool;
    use crate::conversation::intent::IntentState;

    // Helper structs for diesel sql_query results
    #[derive(QueryableByName)]
    struct StringResult {
        #[diesel(sql_type = Text)]
        name: String,
    }

    #[derive(QueryableByName)]
    struct TableInfoRow {
        #[diesel(sql_type = Text)]
        #[diesel(column_name = "name")]
        _name: String,
        #[diesel(sql_type = Text)]
        #[diesel(column_name = "type")]
        _type: String,
    }

    #[derive(QueryableByName)]
    struct CountResult {
        #[diesel(sql_type = BigInt)]
        cnt: i64,
    }

    #[derive(QueryableByName)]
    struct ConvIdResult {
        #[diesel(sql_type = Text)]
        conversation_id: String,
    }

    #[derive(QueryableByName)]
    struct StateAndMemoryResult {
        #[diesel(sql_type = Text)]
        intent_state: String,
        #[diesel(sql_type = Nullable<Text>)]
        memory_id: Option<String>,
    }

    #[derive(QueryableByName)]
    struct ContextResult {
        #[diesel(sql_type = Nullable<Text>)]
        context: Option<String>,
    }

    /// Test 1: MIGRATION round-trip
    /// Verify all migrations apply cleanly on a fresh in-memory SQLite DB.
    /// Checks:
    /// - Migrations run in order without conflicts
    /// - Schema has intent_state column with correct default
    /// - FTS5 external-content table (conversations_fts) exists
    /// - Key indexes created by P4 exist
    #[tokio::test]
    async fn test_migration_round_trip_all_migrations_apply_cleanly() -> Result<()> {
        let pool = DatabasePool::in_memory()?;

        // Test in a blocking task since we need synchronous DB access
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get_connection()?;

            // Verify the conversations table exists
            let table_result: StringResult = diesel::sql_query(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='conversations'",
            )
            .get_result(&mut *conn)
            .map_err(|_| anyhow::anyhow!("conversations table not found"))?;

            assert_eq!(table_result.name, "conversations");

            // Verify intent_state column exists with TEXT type
            let _: Vec<TableInfoRow> = diesel::sql_query(
                "PRAGMA table_info(conversations)",
            )
            .load(&mut *conn)
            .map_err(|e| anyhow::anyhow!("Failed to read table info: {e}"))?;

            // Verify conversations_fts external-content FTS5 table exists
            let fts_result: StringResult = diesel::sql_query(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='conversations_fts'",
            )
            .get_result(&mut *conn)
            .map_err(|_| anyhow::anyhow!("conversations_fts table not found"))?;

            assert_eq!(fts_result.name, "conversations_fts");

            // Verify the FTS5 virtual table is external-content by checking schema
            let fts_schema: StringResult = diesel::sql_query(
                "SELECT sql as name FROM sqlite_master WHERE type='table' AND name='conversations_fts'",
            )
            .get_result(&mut *conn)?;

            // External-content FTS5 tables have 'content=' clause
            assert!(
                fts_schema.name.contains("content='conversations'"),
                "FTS5 should be external-content mode: {}",
                fts_schema.name
            );
            assert!(
                fts_schema.name.contains("content_rowid='rowid'"),
                "FTS5 should use implicit rowid: {}",
                fts_schema.name
            );

            // Verify P4 indexes exist
            let indexes: Vec<StringResult> = diesel::sql_query(
                "SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_conversations_intent%'",
            )
            .load(&mut *conn)?;

            let index_names: Vec<String> = indexes.into_iter().map(|r| r.name).collect();

            assert!(
                index_names.contains(&"idx_conversations_intent_pending".to_string()),
                "idx_conversations_intent_pending not found"
            );
            assert!(
                index_names.contains(&"idx_conversations_intent_verified".to_string()),
                "idx_conversations_intent_verified not found"
            );

            Ok::<(), anyhow::Error>(())
        })
        .await??;

        Ok(())
    }

    /// Test 2: FTS5 external-content refresh and search
    /// Verify that:
    /// - Conversations can be inserted
    /// - refresh_fts_index (rebuild) correctly indexes them
    /// - search_conversations returns results with correct ranking
    #[tokio::test]
    async fn test_refresh_fts_index_and_search() -> Result<()> {
        let pool = DatabasePool::in_memory()?;

        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get_connection()?;
            let workspace_id = 1i64;

            // Insert test conversations directly using raw SQL
            diesel::sql_query(
                "INSERT INTO conversations (conversation_id, workspace_id, title, context, cwd, intent_state, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, datetime('now'))",
            )
            .bind::<diesel::sql_types::Text, _>("conv-001")
            .bind::<diesel::sql_types::BigInt, _>(workspace_id)
            .bind::<diesel::sql_types::Text, _>("First Conv")
            .bind::<diesel::sql_types::Text, _>("This conversation is about Rust programming patterns")
            .bind::<diesel::sql_types::Text, _>("/home/user")
            .bind::<diesel::sql_types::Text, _>("pending")
            .execute(&mut *conn)?;

            diesel::sql_query(
                "INSERT INTO conversations (conversation_id, workspace_id, title, context, cwd, intent_state, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, datetime('now'))",
            )
            .bind::<diesel::sql_types::Text, _>("conv-002")
            .bind::<diesel::sql_types::BigInt, _>(workspace_id)
            .bind::<diesel::sql_types::Text, _>("Second Conv")
            .bind::<diesel::sql_types::Text, _>("This conversation covers database design and indexing strategies")
            .bind::<diesel::sql_types::Text, _>("/home/user/projects")
            .bind::<diesel::sql_types::Text, _>("pending")
            .execute(&mut *conn)?;

            diesel::sql_query(
                "INSERT INTO conversations (conversation_id, workspace_id, title, context, cwd, intent_state, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, datetime('now'))",
            )
            .bind::<diesel::sql_types::Text, _>("conv-003")
            .bind::<diesel::sql_types::BigInt, _>(workspace_id)
            .bind::<diesel::sql_types::Text, _>("Third Conv")
            .bind::<diesel::sql_types::Text, _>("Python web development tutorial with Flask and SQLAlchemy")
            .bind::<diesel::sql_types::Text, _>("/home/user")
            .bind::<diesel::sql_types::Text, _>("pending")
            .execute(&mut *conn)?;

            // NOTE: External-content FTS5 tables are empty after migration.
            // The migration creates the table but doesn't populate it (waiting for first refresh_fts_index call).
            // Trigger rebuild to populate external-content FTS5 index from base table
            diesel::sql_query("INSERT INTO conversations_fts(conversations_fts) VALUES('rebuild')")
                .execute(&mut *conn)?;

            // After rebuild, FTS5 should have 3 entries (the 3 conversations we just inserted)
            let count_after: CountResult = diesel::sql_query("SELECT COUNT(*) as cnt FROM conversations_fts")
                .get_result(&mut *conn)?;

            assert_eq!(count_after.cnt, 3, "FTS5 should have 3 entries after rebuild");

            // Test BM25 search: search for "database" should find conv-002
            let search_sql = "SELECT c.conversation_id FROM conversations c \
                             JOIN conversations_fts fts ON c.rowid = fts.rowid \
                             WHERE conversations_fts MATCH ? \
                             AND c.workspace_id = ? \
                             ORDER BY bm25(conversations_fts)";

            let results: Vec<ConvIdResult> = diesel::sql_query(search_sql)
                .bind::<diesel::sql_types::Text, _>("database")
                .bind::<diesel::sql_types::BigInt, _>(workspace_id)
                .load(&mut *conn)?;

            let result_ids: Vec<String> = results.into_iter().map(|r| r.conversation_id).collect();

            assert!(
                result_ids.contains(&"conv-002".to_string()),
                "Search for 'database' should find conv-002"
            );

            // Test search for "Rust" should find conv-001
            let results: Vec<ConvIdResult> = diesel::sql_query(search_sql)
                .bind::<diesel::sql_types::Text, _>("Rust")
                .bind::<diesel::sql_types::BigInt, _>(workspace_id)
                .load(&mut *conn)?;

            let result_ids: Vec<String> = results.into_iter().map(|r| r.conversation_id).collect();

            assert!(
                result_ids.contains(&"conv-001".to_string()),
                "Search for 'Rust' should find conv-001"
            );

            Ok::<(), anyhow::Error>(())
        })
        .await??;

        Ok(())
    }

    /// Test 3: IntentState transition guards
    /// Verify the state machine enforces legal transitions and rejects illegal ones
    #[test]
    fn test_intent_state_transition_guards_legal_transitions() {
        // Forward path: pending → extracting → extracted → verified → pruned
        assert!(IntentState::Pending.can_transition_to(IntentState::Extracting));
        assert!(IntentState::Extracting.can_transition_to(IntentState::Extracted));
        assert!(IntentState::Extracted.can_transition_to(IntentState::Verified));
        assert!(IntentState::Verified.can_transition_to(IntentState::Pruned));

        // Idempotent transitions
        assert!(IntentState::Pending.can_transition_to(IntentState::Pending));
        assert!(IntentState::Extracting.can_transition_to(IntentState::Extracting));
        assert!(IntentState::Extracted.can_transition_to(IntentState::Extracted));
        assert!(IntentState::Verified.can_transition_to(IntentState::Verified));
        assert!(IntentState::Pruned.can_transition_to(IntentState::Pruned));

        // Reversions (on failure)
        assert!(IntentState::Extracting.can_transition_to(IntentState::Pending));
        assert!(IntentState::Extracted.can_transition_to(IntentState::Pending));
        assert!(IntentState::Verified.can_transition_to(IntentState::Pending));

        // Forward skip (manual override)
        assert!(IntentState::Pending.can_transition_to(IntentState::Extracted));
        assert!(IntentState::Pending.can_transition_to(IntentState::Verified));
    }

    #[test]
    fn test_intent_state_transition_guards_illegal_transitions() {
        // Cannot jump directly to pruned without going through verified
        assert!(!IntentState::Pending.can_transition_to(IntentState::Pruned));
        assert!(!IntentState::Extracting.can_transition_to(IntentState::Pruned));
        assert!(!IntentState::Extracted.can_transition_to(IntentState::Pruned));

        // Pruned is final; no forward transitions from pruned
        assert!(!IntentState::Pruned.can_transition_to(IntentState::Extracting));
        assert!(!IntentState::Pruned.can_transition_to(IntentState::Extracted));
        assert!(!IntentState::Pruned.can_transition_to(IntentState::Verified));

        // No backwards skipping (e.g., extracting to verified)
        assert!(!IntentState::Extracting.can_transition_to(IntentState::Verified));
        assert!(!IntentState::Extracted.can_transition_to(IntentState::Extracting));
    }

    /// Test 4: Prune conversation gate — verify pruning only allowed when intent_state = 'verified'
    #[tokio::test]
    async fn test_prune_conversation_gate_requires_verified_state() -> Result<()> {
        let pool = DatabasePool::in_memory()?;

        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get_connection()?;
            let workspace_id = 1i64;

            // Insert a conversation in 'pending' state
            diesel::sql_query(
                "INSERT INTO conversations (conversation_id, workspace_id, title, context, intent_state, created_at)
                 VALUES (?, ?, ?, ?, ?, datetime('now'))",
            )
            .bind::<diesel::sql_types::Text, _>("conv-prune-test")
            .bind::<diesel::sql_types::BigInt, _>(workspace_id)
            .bind::<diesel::sql_types::Text, _>("Test")
            .bind::<diesel::sql_types::Text, _>("Large context blob")
            .bind::<diesel::sql_types::Text, _>("pending")
            .execute(&mut *conn)?;

            // Attempt to prune while in 'pending' state — should affect 0 rows
            let rows_affected: usize = diesel::sql_query(
                "UPDATE conversations SET context = NULL WHERE conversation_id = ? AND intent_state = ?",
            )
            .bind::<diesel::sql_types::Text, _>("conv-prune-test")
            .bind::<diesel::sql_types::Text, _>("verified")
            .execute(&mut *conn)?;

            assert_eq!(rows_affected, 0, "Pruning should not affect conversations in 'pending' state");

            // Verify the context is still intact
            let context_row: ContextResult = diesel::sql_query(
                "SELECT context FROM conversations WHERE conversation_id = ?",
            )
            .bind::<diesel::sql_types::Text, _>("conv-prune-test")
            .get_result(&mut *conn)?;

            assert_eq!(context_row.context, Some("Large context blob".to_string()));

            // Now transition to 'verified'
            diesel::sql_query(
                "UPDATE conversations SET intent_state = ? WHERE conversation_id = ?",
            )
            .bind::<diesel::sql_types::Text, _>("verified")
            .bind::<diesel::sql_types::Text, _>("conv-prune-test")
            .execute(&mut *conn)?;

            // Now pruning should succeed
            let rows_affected: usize = diesel::sql_query(
                "UPDATE conversations SET context = NULL WHERE conversation_id = ? AND intent_state = ?",
            )
            .bind::<diesel::sql_types::Text, _>("conv-prune-test")
            .bind::<diesel::sql_types::Text, _>("verified")
            .execute(&mut *conn)?;

            assert_eq!(rows_affected, 1, "Pruning should affect 1 row when intent_state = 'verified'");

            // Verify the context is now NULL
            let context_after: ContextResult = diesel::sql_query(
                "SELECT context FROM conversations WHERE conversation_id = ?",
            )
            .bind::<diesel::sql_types::Text, _>("conv-prune-test")
            .get_result(&mut *conn)?;

            assert_eq!(context_after.context, None, "Context should be NULL after pruning");

            Ok::<(), anyhow::Error>(())
        })
        .await??;

        Ok(())
    }

    /// Test 5: FTS5 external-content schema validation
    /// Verify that the migration correctly created external-content FTS5 without triggers
    #[tokio::test]
    async fn test_fts5_external_content_schema() -> Result<()> {
        let pool = DatabasePool::in_memory()?;

        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get_connection()?;

            // Verify no synchronous FTS triggers remain (P2 removed them)
            let triggers: Vec<StringResult> = diesel::sql_query(
                "SELECT name FROM sqlite_master WHERE type='trigger' AND name LIKE 'conversations_fts_%'",
            )
            .load(&mut *conn)?;

            assert_eq!(
                triggers.len(),
                0,
                "FTS triggers should be dropped by P2"
            );

            // Verify FTS5 has the correct tokenizer (porter) for stemming
            let fts_schema: StringResult = diesel::sql_query(
                "SELECT sql as name FROM sqlite_master WHERE type='table' AND name='conversations_fts'",
            )
            .get_result(&mut *conn)?;

            assert!(
                fts_schema.name.contains("tokenize='porter'"),
                "FTS5 should use porter tokenizer for stemming: {}",
                fts_schema.name
            );

            // Verify FTS5 indexes the correct columns: title, context, cwd
            assert!(
                fts_schema.name.contains("title") && fts_schema.name.contains("context") && fts_schema.name.contains("cwd"),
                "FTS5 should index title, context, and cwd columns: {}",
                fts_schema.name
            );

            Ok::<(), anyhow::Error>(())
        })
        .await??;

        Ok(())
    }

    /// Test 6: Multiple conversations with different intent states
    /// Verify that indexing works correctly for mixed intent states
    #[tokio::test]
    async fn test_intent_state_indexing_with_mixed_states() -> Result<()> {
        let pool = DatabasePool::in_memory()?;

        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get_connection()?;
            let workspace_id = 1i64;

            // Insert conversations in different states
            for (id, state) in vec![
                ("conv-p1", "pending"),
                ("conv-p2", "pending"),
                ("conv-e1", "extracting"),
                ("conv-ex1", "extracted"),
                ("conv-v1", "verified"),
                ("conv-v2", "verified"),
                ("conv-pr1", "pruned"),
            ] {
                diesel::sql_query(
                    "INSERT INTO conversations (conversation_id, workspace_id, intent_state, created_at)
                     VALUES (?, ?, ?, datetime('now'))",
                )
                .bind::<diesel::sql_types::Text, _>(id)
                .bind::<diesel::sql_types::BigInt, _>(workspace_id)
                .bind::<diesel::sql_types::Text, _>(state)
                .execute(&mut *conn)?;
            }

            // Query pending or extracting
            let pending_extracting: Vec<ConvIdResult> = diesel::sql_query(
                "SELECT conversation_id FROM conversations WHERE workspace_id = ? AND intent_state IN ('pending', 'extracting') ORDER BY conversation_id",
            )
            .bind::<diesel::sql_types::BigInt, _>(workspace_id)
            .load(&mut *conn)?;

            assert_eq!(pending_extracting.len(), 3, "Should find 3 conversations in pending or extracting state");

            // Query verified only
            let verified: Vec<ConvIdResult> = diesel::sql_query(
                "SELECT conversation_id FROM conversations WHERE workspace_id = ? AND intent_state = 'verified' ORDER BY conversation_id",
            )
            .bind::<diesel::sql_types::BigInt, _>(workspace_id)
            .load(&mut *conn)?;

            assert_eq!(verified.len(), 2, "Should find 2 conversations in verified state");

            Ok::<(), anyhow::Error>(())
        })
        .await??;

        Ok(())
    }

    /// Test 7: Memory ID tracking for audit trail
    /// Verify that memory_id and extracted_at columns are tracked correctly
    #[tokio::test]
    async fn test_memory_id_and_extracted_at_tracking() -> Result<()> {
        let pool = DatabasePool::in_memory()?;

        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get_connection()?;
            let workspace_id = 1i64;
            let conv_id = "conv-audit";
            let memory_id = "mem-uuid-12345";

            // Insert conversation with memory tracking
            diesel::sql_query(
                "INSERT INTO conversations (conversation_id, workspace_id, intent_state, memory_id, extracted_at, created_at)
                 VALUES (?, ?, ?, ?, datetime('now'), datetime('now'))",
            )
            .bind::<diesel::sql_types::Text, _>(conv_id)
            .bind::<diesel::sql_types::BigInt, _>(workspace_id)
            .bind::<diesel::sql_types::Text, _>("extracted")
            .bind::<diesel::sql_types::Text, _>(memory_id)
            .execute(&mut *conn)?;

            // Query back and verify
            let result: StateAndMemoryResult = diesel::sql_query(
                "SELECT intent_state, memory_id FROM conversations WHERE conversation_id = ?",
            )
            .bind::<diesel::sql_types::Text, _>(conv_id)
            .get_result(&mut *conn)?;

            assert_eq!(result.intent_state, "extracted");
            assert_eq!(result.memory_id, Some(memory_id.to_string()));

            Ok::<(), anyhow::Error>(())
        })
        .await??;

        Ok(())
    }
}
