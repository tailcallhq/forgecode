use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use url::Url;

use crate::{
    AnyProvider, AuthCredential, ChatCompletionMessage, Context, Conversation, ConversationId,
    ConversationSummary, MigrationResult, Model, ModelId, Provider, ProviderId, ProviderTemplate,
    ResultStream, SearchMatch, Skill, Snapshot, WorkspaceAuth, WorkspaceId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPatchBlock {
    pub patch: String,
    pub patched_text: String,
}

/// Result of importing conversations from a foreign forge installation.
///
/// Rows that were parsed and written count toward `imported`. Rows skipped
/// (no context blob / already-empty shells) count toward `skipped`. Rows
/// that failed to parse or write count toward `errors` and are not aborted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForgeImportResult {
    pub imported: usize,
    pub skipped: usize,
    pub errors: usize,
}

/// Result of a one-way import from an official forge-lineage database.
///
/// The source database is opened read-only and is never modified. Rows whose
/// `conversation_id` already exists in the destination repository are skipped,
/// which makes re-running the import idempotent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForgeImportReport {
    /// Conversations read from the source database.
    pub source_total: usize,
    /// Conversations written into the destination repository.
    pub imported: usize,
    /// Conversations skipped because their ID already exists.
    pub skipped_existing: usize,
    /// Rows skipped because `conversation_id` was not parseable.
    pub invalid_id: usize,
    /// Conversations imported without a context blob because the source
    /// context could not be parsed into the heliosLite schema.
    pub context_parse_failed: usize,
    /// Rows skipped due to insert or read errors.
    pub errors: usize,
    /// When `dry_run` was set, the report describes what *would* have been
    /// written but no inserts were performed.
    pub dry_run: bool,
}

/// Options that tune the behaviour of
/// [`ConversationRepository::import_forge_db`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForgeImportOptions {
    /// When set, the source DB is scanned exactly as for a real import but
    /// no rows are inserted. The returned [`ForgeImportReport`] reflects what
    /// would have been written.
    pub dry_run: bool,
    /// Print each row's outcome (imported / skipped / failed) to stderr as
    /// the import progresses. Useful for very large source databases.
    pub verbose: bool,
}

/// Result of a one-way export from a heliosLite database to a freshly-created
/// official-schema SQLite file.
///
/// Compression (`context_zstd`) is reversed: the resulting DB has plain
/// `context` blobs readable by the official lineage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForgeExportReport {
    /// Conversations read from this heliosLite repository.
    pub source_total: usize,
    /// Conversations written into the destination DB.
    pub exported: usize,
    /// Rows skipped because decompression failed.
    pub decompression_failed: usize,
    /// Rows skipped due to write errors.
    pub errors: usize,
    /// When `dry_run` was set, no DB file was created.
    pub dry_run: bool,
}

/// Options that tune the behaviour of
/// [`ConversationRepository::export_forge_db`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForgeExportOptions {
    /// When set, the export scans the same rows but does not create the
    /// destination DB. The report reflects what *would* have been
    /// written.
    pub dry_run: bool,
    /// Output format. Defaults to `Sqlite` (mirrors upstream `forge.db`
    /// schema). `Jsonl` and `Csv` write one record per line to the
    /// destination path and are useful for off-system consumption.
    pub format: ForgeExportFormat,
    /// By default, agent-launched rows are skipped from the export (the
    /// TUI picker hides them, so exporting them is rarely useful). Set
    /// `include_agent` to `true` to include them.
    pub include_agent: bool,
}

/// Output format for [`ConversationRepository::export_forge_db`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ForgeExportFormat {
    /// SQLite database mirroring the upstream `forge` schema (default).
    #[default]
    Sqlite,
    /// Newline-delimited JSON: one `[title,id,created_at,updated_at,context]`
    /// tuple per row. `context` is a JSON string (not an object) so the
    /// downstream parser can re-parse it transparently.
    Jsonl,
    /// CSV with header: `conversation_id,title,created_at,updated_at,context`.
    /// Context is emitted as a single field, double-quote-escaped.
    Csv,
}
/// Aggregate database statistics surfaced by `heliosdoctor --verbose`.
///
/// Used to spot compression regressions (compressed rows missing their
/// `context_zstd` payload), oversized contexts, and agent-batch fanout.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeliosdoctorDbStats {
    /// Total conversation rows in the database.
    pub total_conversations: u64,
    /// Rows where `is_compressed = 1` (context lives in `context_zstd`).
    pub compressed_rows: u64,
    /// Rows where `is_compressed = 0` (plain `context` column populated).
    pub uncompressed_rows: u64,
    /// Rows where `context IS NULL` and `is_compressed = 0` (empty shell).
    pub empty_rows: u64,
    /// Rows whose context blob is over 1 MB (decompression / render cost).
    pub oversized_rows: u64,
    /// Agent-launched rows (`context.initiator = "agent"`).
    pub agent_initiated_rows: u64,
    /// `PRAGMA integrity_check` result (`"ok"` when healthy).
    pub integrity_check: String,
    /// Whether the legacy DB was successfully ATTACHed for read-fallback
    /// (only meaningful when split-DB is active).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub legacy_attached: Option<bool>,
    /// Write-side DB path (the file the current binary writes to).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub write_db_path: Option<String>,
    /// Legacy read-side DB path (ATTACHed read-only when present).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub legacy_db_path: Option<String>,
    /// Per-table row counts when split-DB is active.
    /// `tables["conversations"] = (write_count, legacy_count)`, etc.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub tables: BTreeMap<String, (Option<i64>, Option<i64>)>,
    /// Error string when stats collection failed (kept for the operator).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

/// Filter for `forget_conversations`. At least one selector must be set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForgeForgetOptions {
    /// Delete by exact conversation_id. Any ID that does not match is silently
    /// ignored (idempotent).
    pub ids: Vec<ConversationId>,
    /// Delete all rows whose `source` column equals this value
    /// (e.g. `imported:forge`, `agent`, `user`).
    pub source: Option<String>,
    /// Delete rows where `updated_at` is older than `now - older_than_secs`.
    pub older_than_secs: Option<i64>,
    /// When `true`, perform a no-op scan and report the count that *would*
    /// be deleted. The database is **not** modified.
    pub dry_run: bool,
}

/// Result of `forget_conversations`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForgeForgetReport {
    /// Number of rows that matched the filter.
    pub matched: usize,
    /// Number of rows actually removed. Equal to `matched` when
    /// `dry_run` is `false`.
    pub deleted: usize,
    /// When `dry_run` was set, no rows were deleted.
    pub dry_run: bool,
}

/// Options that tune [`ConversationRepository::migrate_data_dir`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrateOptions {
    /// When set, the migration is computed but no files are moved or
    /// renamed. The returned [`ForgeMigrateReport`] describes what
    /// *would* have happened.
    pub dry_run: bool,
}

/// Outcome of `migrate_data_dir`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForgeMigrateReport {
    /// Resolved source (`~/.forge`) and destination (`~/.helioslite`) paths.
    pub source_path: PathBuf,
    pub destination_path: PathBuf,
    /// High-level outcome (one of `migrated`, `already_migrated`,
    /// `noop_legacy_missing`). The detailed boolean flags below are
    /// provided for tool consumption.
    pub outcome: String,
    /// Total bytes copied (from the source DB file).
    pub bytes_copied: u64,
    /// Number of conversations confirmed readable in the copied DB.
    pub conversations_verified: u64,
    /// If `migrated`, the legacy directory was renamed to
    /// `~/.forge.migrated-YYYYMMDDHHMMSS`. The new path is recorded here.
    pub renamed_legacy_to: Option<PathBuf>,
}

/// Alias used by the CLI layer; canonical name is `ForgeMigrateReport`.
pub type MigrateReport = ForgeMigrateReport;

/// Repository for managing file snapshots
///
/// This repository provides operations for creating and restoring file
/// snapshots, enabling undo functionality for file modifications.
#[async_trait::async_trait]
pub trait SnapshotRepository: Send + Sync {
    /// Inserts a new snapshot for the given file path
    ///
    /// # Arguments
    /// * `file_path` - Path to the file to snapshot
    ///
    /// # Errors
    /// Returns an error if the snapshot creation fails
    async fn insert_snapshot(&self, file_path: &Path) -> Result<Snapshot>;

    /// Restores the most recent snapshot for the given file path
    ///
    /// # Arguments
    /// * `file_path` - Path to the file to restore
    ///
    /// # Errors
    /// Returns an error if no snapshot exists or restoration fails
    async fn undo_snapshot(&self, file_path: &Path) -> Result<()>;
}

/// Repository for managing conversation persistence
///
/// This repository provides CRUD operations for conversations, including
/// creating, retrieving, and listing conversations.
#[async_trait::async_trait]
pub trait ConversationRepository: Send + Sync {
    /// Creates or updates a conversation from a borrowed reference, avoiding
    /// the per-call `Conversation` clone on hot paths (orchestrator loop,
    /// service `modify_conversation`).
    ///
    /// This is the preferred variant for code that already holds a
    /// `&Conversation` (i.e. almost every caller in the orchestrator).
    /// The legacy by-value [`Self::upsert_conversation`] is preserved for
    /// back-compat with code that owns the conversation outright.
    ///
    /// # Arguments
    /// * `conversation` - Borrowed conversation to persist
    ///
    /// # Errors
    /// Returns an error if the operation fails
    async fn upsert_conversation_ref(&self, conversation: &Conversation) -> Result<()>;

    /// Creates or updates a conversation
    ///
    /// # Arguments
    /// * `conversation` - The conversation to persist
    ///
    /// # Errors
    /// Returns an error if the operation fails
    async fn upsert_conversation(&self, conversation: Conversation) -> Result<()>;

    /// Retrieves a conversation by its ID
    ///
    /// # Arguments
    /// * `conversation_id` - The ID of the conversation to retrieve
    ///
    /// # Errors
    /// Returns an error if the operation fails
    async fn get_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<Conversation>>;

    /// Retrieves all conversations with an optional limit
    ///
    /// # Arguments
    /// * `limit` - Optional maximum number of conversations to retrieve
    ///
    /// # Errors
    /// Returns an error if the operation fails
    async fn get_all_conversations(
        &self,
        limit: Option<usize>,
    ) -> Result<Option<Vec<Conversation>>>;

    /// Retrieves the most recent conversation
    ///
    /// # Errors
    /// Returns an error if the operation fails
    async fn get_last_conversation(&self) -> Result<Option<Conversation>>;

    /// Permanently deletes a conversation
    ///
    /// # Arguments
    /// * `conversation_id` - The ID of the conversation to delete
    ///
    /// # Errors
    /// Returns an error if the operation fails
    async fn delete_conversation(&self, conversation_id: &ConversationId) -> Result<()>;

    /// Retrieves all conversations that have the given parent_id
    ///
    /// # Arguments
    /// * `parent_id` - The ID of the parent conversation
    ///
    /// # Errors
    /// Returns an error if the operation fails
    async fn get_conversations_by_parent(
        &self,
        parent_id: &ConversationId,
    ) -> Result<Option<Vec<Conversation>>>;

    /// Retrieves all top-level conversations (those without a parent_id)
    ///
    /// # Arguments
    /// * `limit` - Optional maximum number of conversations to retrieve
    ///
    /// # Errors
    /// Returns an error if the operation fails
    async fn get_parent_conversations(
        &self,
        limit: Option<usize>,
    ) -> Result<Option<Vec<Conversation>>>;

    /// Lightweight variant of [`get_parent_conversations`] that selects only
    /// metadata columns (`conversation_id`, `title`, `created_at`,
    /// `updated_at`, `parent_id`, `message_count`, `cwd`) and returns
    /// [`ConversationSummary`] items. This avoids loading the multi-MB
    /// `context` / `context_zstd` blobs and the subsequent zstd
    /// decompression + JSON deserialisation of every conversation row.
    ///
    /// Use this for the TUI conversation list selector; use
    /// [`get_parent_conversations`] only when the full `Context` is needed
    /// (e.g. conversation open / clone).
    ///
    /// # Arguments
    /// * `limit` - Optional maximum number of conversations to retrieve
    ///
    /// # Errors
    /// Returns an error if the operation fails
    async fn get_parent_conversations_lite(
        &self,
        limit: Option<usize>,
        all_workspaces: bool,
    ) -> Result<Option<Vec<ConversationSummary>>>;

    /// Retrieves conversations by source (e.g., "interactive", "headless",
    /// "forge-p")
    ///
    /// # Arguments
    /// * `source` - The source to filter by
    /// * `limit` - Optional maximum number of conversations to retrieve
    ///
    /// # Errors
    /// Returns an error if the operation fails
    async fn get_conversations_by_source(
        &self,
        source: &str,
        limit: Option<usize>,
    ) -> Result<Option<Vec<Conversation>>>;

    /// Full-text search over conversation titles and context, scoped to the
    /// current workspace. Backed by the FTS5 virtual table installed by
    /// migration `2026-06-14-000002_add_fts5_to_conversations`.
    ///
    /// Results are ranked by BM25 (`fts.rank`). An empty `Vec` means the
    /// query matched zero rows (use `.is_empty()` on the result).
    ///
    /// # Arguments
    /// * `query` - FTS5 MATCH expression (e.g. `"rust refactor"`, `"tokio*"`).
    ///   Caller is responsible for sanitising; the implementation passes it
    ///   through to SQLite unchanged.
    /// * `limit` - Optional cap on returned rows.
    ///
    /// # Errors
    /// Returns an error if the FTS query is malformed or the database call
    /// fails.
    async fn search_conversations(
        &self,
        query: &str,
        limit: Option<usize>,
    ) -> Result<Vec<Conversation>>;

    /// Returns a short FTS5 snippet (~32 tokens) for a single
    /// `(conversation_id, query)` pair, with the matched terms wrapped in
    /// `[…]` and the surrounding text wrapped in `…`. Used by the UI to
    /// render a "matched passage" preview for the currently selected
    /// search hit without forcing the main search query to include the
    /// snippet column (which would couple the row layout to
    /// `ConversationRecord`).
    ///
    /// Returns `Ok(None)` when the query does not match that conversation
    /// — callers should treat `None` as "no preview available" and fall
    /// back to the conversation title.
    ///
    /// # Errors
    /// Returns an error if the FTS query is malformed or the database
    /// call fails.
    async fn get_conversation_snippet(
        &self,
        conversation_id: &ConversationId,
        query: &str,
        token_count: usize,
    ) -> Result<Option<String>>;

    /// Reclaims FTS5 segment shadow data by running
    /// `INSERT INTO conversations_fts(conversations_fts) VALUES('optimize')`.
    ///
    /// FTS5 maintains per-segment shadow trees that can grow unboundedly under
    /// heavy write / delete workloads. Periodically calling `optimize` (e.g.
    /// at the end of a long session or from a maintenance command) compacts
    /// them back into a single segment, reducing query-time shadow-walk cost
    /// and disk footprint.
    ///
    /// # Errors
    /// Returns an error if the optimize statement fails to execute.
    async fn optimize_fts_index(&self) -> Result<()>;

    /// Rebuilds the contentful FTS5 index from the current conversation
    /// rows without touching the hot write path.
    ///
    /// The refresh uses the FTS5-native `delete-all` command to clear the
    /// index, then repopulates it from `conversations` in a single
    /// transaction so callers can run it on a maintenance cadence.
    ///
    /// # Errors
    /// Returns an error if either FTS5 statement fails to execute.
    async fn refresh_fts_index(&self) -> Result<()>;

    /// Re-binds a subagent conversation to a different parent. Pass `None`
    /// for `new_parent_id` to detach the conversation entirely (promotes it
    /// to a top-level session).
    ///
    /// The existing `parent_id` (if any) is replaced atomically; no other
    /// columns are touched. This does not recurse into descendants —
    /// subagents of the reparented conversation remain linked to *this*
    /// conversation.
    ///
    /// # Arguments
    /// * `conversation_id` - The conversation to reparent.
    /// * `new_parent_id` - The new parent, or `None` to detach.
    ///
    /// # Errors
    /// Returns an error if the update fails or the conversation does not
    /// exist.
    async fn update_parent_id(
        &self,
        conversation_id: &ConversationId,
        new_parent_id: Option<&ConversationId>,
    ) -> Result<()>;

    /// Retrieves conversations by working directory (cwd).
    ///
    /// Used by the session viewer to scope by cwd (per-project filtering).
    /// The match is an exact equality on the `cwd` column, not a fuzzy
    /// search — combine with [`Self::search_conversations`] for substring
    /// matching.
    ///
    /// # Arguments
    /// * `cwd` - Exact cwd to match.
    /// * `limit` - Optional cap on returned rows.
    ///
    /// # Errors
    /// Returns an error if the query fails.
    async fn get_conversations_by_cwd(
        &self,
        cwd: &str,
        limit: Option<usize>,
    ) -> Result<Option<Vec<Conversation>>>;

    /// Updates the intent_state of a conversation with state machine
    /// enforcement.
    ///
    /// ADR-103: Intent-gated semantic pruning. Validates that the transition
    /// from the current state to the new state is allowed before updating.
    /// Rejects illegal transitions (e.g., trying to prune before verified).
    ///
    /// # Arguments
    /// * `conversation_id` - The conversation to update
    /// * `new_state` - The target intent state (as string: "pending",
    ///   "extracting", etc.)
    ///
    /// # Errors
    /// Returns an error if:
    /// - The conversation doesn't exist
    /// - The transition from current state to new_state is forbidden
    /// - The database update fails
    async fn mark_intent_state(
        &self,
        conversation_id: &ConversationId,
        new_state: &str,
    ) -> Result<()>;

    /// Lists conversations eligible for pruning (intent_state='verified').
    ///
    /// Returns up to `limit` conversations ordered by blob size (largest first)
    /// to maximize space reclaimed. Used by the pruning batch operator.
    ///
    /// # Arguments
    /// * `workspace_id` - Filter by workspace (optional; if provided, scopes
    ///   search)
    /// * `limit` - Maximum number of rows to return
    ///
    /// # Errors
    /// Returns an error if the query fails.
    async fn list_prune_eligible(
        &self,
        workspace_id: Option<i64>,
        limit: usize,
    ) -> Result<Vec<Conversation>>;

    /// Marks a conversation as pruned by compressing its context blob.
    ///
    /// ADR-103: Pruning is only allowed if current intent_state='verified'.
    /// Replaces the context blob with a compact JSON summary and sets
    /// intent_state='pruned'. The summary preserves just enough metadata
    /// for the conversation to remain queryable without the full blob.
    ///
    /// # Arguments
    /// * `conversation_id` - The conversation to prune
    ///
    /// # Errors
    /// Returns an error if:
    /// - The conversation doesn't exist
    /// - Current intent_state != 'verified' (safety guard)
    /// - The database update fails
    async fn prune_conversation(&self, conversation_id: &ConversationId) -> Result<()>;

    /// Rewinds a conversation to the snapshot recorded at the last
    /// compaction point. Used by the `/rewind` slash command (Claude
    /// Code parity) to roll back the conversation to its pre-compaction
    /// state.
    ///
    /// Implementation strategy: persists a `compaction_anchor` row
    /// whenever the user runs `/compact` (a copy of the conversation
    /// JSON before compaction). On rewind, the repo reads the most
    /// recent anchor for `conversation_id` and replaces the live
    /// conversation's content with it.
    ///
    /// # Arguments
    /// * `conversation_id` - The conversation to rewind.
    ///
    /// # Returns
    /// * `Ok(Some(Conversation))` with the restored conversation if an anchor
    ///   exists.
    /// * `Ok(None)` if no anchor has ever been recorded (rewind is a no-op in
    ///   that case).
    ///
    /// # Errors
    /// Returns an error if the anchor read or the conversation update
    /// fails.
    async fn rewind_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<Conversation>>;

    /// Idempotent maintenance command: zstd-compresses all rows where
    /// `is_compressed = 0` and `context IS NOT NULL`.
    ///
    /// Safe to run at any time — rows already compressed (`is_compressed = 1`)
    /// are skipped. Rows with a NULL context column are skipped. Progress is
    /// reported per-row via the returned `(compressed, skipped, errors)` tuple.
    ///
    /// **Not** run automatically on startup; invoke explicitly via
    /// `forge maintenance compress`.
    ///
    /// # Returns
    /// `(compressed, skipped, errors)` counts.
    ///
    /// # Errors
    /// Returns an error only if the batch query itself fails. Per-row
    /// compression errors are counted in `errors` and do not abort the batch.
    async fn compress_uncompressed_contexts(&self) -> Result<(usize, usize, usize)>;

    /// One-way import of conversations from an official forge-lineage SQLite
    /// database (plain `context` schema, no zstd compression columns) into
    /// this repository.
    ///
    /// The source database is opened read-only and is never modified.
    /// Conversations whose `conversation_id` already exists in this
    /// repository are skipped, making the operation idempotent. Rows whose
    /// context cannot be parsed into the heliosLite schema are imported
    /// without a context blob and reported via [`ForgeImportReport`].
    ///
    /// # Errors
    /// Returns an error if the source file is missing, is not a forge
    /// conversations database, or if it is already a heliosLite/fork-schema
    /// database (nothing to import).
    async fn import_forge_db(&self, source: PathBuf) -> Result<ForgeImportReport>;

    /// One-way import with explicit [`ForgeImportOptions`].
    ///
    /// When `options.dry_run` is `true`, the source DB is scanned exactly
    /// as for a real import but no rows are inserted. The returned
    /// [`ForgeImportReport`] reflects what *would* have been written.
    ///
    /// When `options.verbose` is `true`, each row's outcome is logged
    /// (imported / skipped / failed) for visibility on large source DBs.
    ///
    /// The inserts are wrapped in a single SQLite transaction so a
    /// partial import cannot leave the destination in an inconsistent
    /// state.
    ///
    /// # Errors
    /// Returns an error if the source file is missing, is not a forge
    /// conversations database, or if it is already a heliosLite/fork-schema
    /// database (nothing to import). A failure inside the transaction
    /// also aborts the entire batch.
    async fn import_forge_db_with_options(
        &self,
        source: PathBuf,
        options: &ForgeImportOptions,
    ) -> Result<ForgeImportReport>;

    /// One-way export of conversations from this heliosLite repository to a
    /// freshly-created official-schema SQLite file at `destination`.
    ///
    /// The destination DB is created (parents included); any existing file
    /// at the path is replaced. The schema matches the official forge
    /// lineage (no `is_compressed`, no `context_zstd`, plain `context`
    /// column). Compressed rows are decompressed during the export.
    ///
    /// This is the inverse of [`import_forge_db`]: it lets you hand off a
    /// heliosLite DB to the official lineage.
    ///
    /// # Errors
    /// Returns an error if `destination` cannot be created or if a row
    /// cannot be decompressed / written.
    async fn export_forge_db(
        &self,
        destination: PathBuf,
        options: &ForgeExportOptions,
    ) -> Result<ForgeExportReport>;

    /// Aggregate DB stats for `heliosdoctor --verbose`.
    ///
    /// Includes compression health, oversized context count, agent
    /// fanout, and a `PRAGMA integrity_check` result. Implementations
    /// should execute the counts in a single SQLite query when possible
    /// to keep this cheap on large DBs.
    async fn database_stats(&self) -> Result<HeliosdoctorDbStats>;

    /// Remove conversations matching the supplied filter.
    ///
    /// At least one of `ids` / `source` / `older_than_secs` must be set;
    /// calling with all `None` is an error (prevents accidental full-delete).
    /// The match is exact, case-sensitive, and applies to the row's
    /// `source` column (e.g. `imported:forge`, `agent`, `user`).
    ///
    /// Set `rows_affected` in the returned report. Deleted rows are
    /// removed from `conversations` (and dependent rows via foreign keys).
    /// Snapshots / intent_state rows for the deleted conversations are
    /// left intentionally — they are inert and can be cleaned by a future
    /// `forge maintenance` sweep.
    ///
    /// # Errors
    /// Returns an error if no filter is provided, the database update
    /// fails, or the resolved DB is on a read-only volume.
    async fn forget_conversations(&self, options: &ForgeForgetOptions)
    -> Result<ForgeForgetReport>;

    /// Atomically migrate the active data directory from `~/.forge` to
    /// `~/.helioslite` (the canonical heliosLite location).
    ///
    /// Returns [`ForgeMigrateReport`] describing what was moved. The
    /// operation is idempotent: if `~/.helioslite` already exists and
    /// contains data, the function reports `already_migrated` and exits
    /// without touching anything. If `~/.forge` is missing, the result
    /// is `noop_legacy_missing` and the canonical directory is created
    /// empty (so the launcher treats the install as fresh).
    ///
    /// The DB is copied (not moved) to `~/.helioslite/.forge.db` and
    /// then validated by reopening it. Only after the copy is verified
    /// does the function rename the legacy `~/.forge` directory to
    /// `~/.forge.migrated-YYYYMMDDHHMMSS` so the user can roll back.
    ///
    /// # Errors
    /// Returns an error if the source DB is unreadable, the copy fails,
    /// or the post-copy validation fails.
    async fn migrate_data_dir(&self, options: &MigrateOptions) -> Result<ForgeMigrateReport>;
}

/// Environment diagnostics produced by `heliosdoctor`.
///
/// `config_source` describes where the base path was resolved from:
/// `override-env` (FORGE_CONFIG), `helioslite` (canonical ~/.helioslite),
/// `legacy-forge` (read-in-place ~/.forge), or `default` (fresh install).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeliosdoctorInfo {
    pub version: String,
    pub binary_stem: String,
    pub base_path: PathBuf,
    pub db_path: PathBuf,
    pub updater_repo: String,
    pub updater_binary: String,
    pub config_source: String,
    /// Populated only when `--verbose` is requested. Reports compression
    /// health, agent fanout, oversized contexts, and a DB integrity check.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub db_stats: Option<HeliosdoctorDbStats>,
    /// Write-side DB path when split-DB is active. `None` when both paths
    /// resolve to the same file (the historical single-DB layout).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub write_db_path: Option<PathBuf>,
    /// Legacy read-side DB path when split-DB is active. `None` when the
    /// operator hasn't set `FORGE_LEGACY_DB_PATH` / no legacy file exists.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub legacy_db_path: Option<PathBuf>,
}

#[async_trait::async_trait]
pub trait ChatRepository: Send + Sync {
    async fn chat(
        &self,
        model_id: &ModelId,
        context: Context,
        provider: Provider<Url>,
    ) -> ResultStream<ChatCompletionMessage, anyhow::Error>;
    async fn models(&self, provider: Provider<Url>) -> anyhow::Result<Vec<Model>>;
}

#[async_trait::async_trait]
pub trait ProviderRepository: Send + Sync {
    async fn get_all_providers(&self) -> anyhow::Result<Vec<AnyProvider>>;
    async fn get_provider(&self, id: ProviderId) -> anyhow::Result<ProviderTemplate>;
    async fn upsert_credential(&self, credential: AuthCredential) -> anyhow::Result<()>;
    async fn get_credential(&self, id: &ProviderId) -> anyhow::Result<Option<AuthCredential>>;
    async fn remove_credential(&self, id: &ProviderId) -> anyhow::Result<()>;
    async fn migrate_env_credentials(&self) -> anyhow::Result<Option<MigrationResult>>;
}

/// Repository for managing workspace indexing and search operations
#[async_trait::async_trait]
pub trait WorkspaceIndexRepository: Send + Sync {
    /// Authenticate with the indexing service via gRPC API
    async fn authenticate(&self) -> anyhow::Result<WorkspaceAuth>;

    /// Create a new workspace on the indexing server
    async fn create_workspace(
        &self,
        working_dir: &std::path::Path,
        auth_token: &crate::ApiKey,
    ) -> anyhow::Result<WorkspaceId>;

    /// Upload files to be indexed
    async fn upload_files(
        &self,
        upload: &crate::FileUpload,
        auth_token: &crate::ApiKey,
    ) -> anyhow::Result<crate::FileUploadInfo>;

    /// Search the indexed codebase using semantic search
    async fn search(
        &self,
        query: &crate::CodeSearchQuery<'_>,
        auth_token: &crate::ApiKey,
    ) -> anyhow::Result<Vec<crate::Node>>;

    /// List all workspaces for a user
    async fn list_workspaces(
        &self,
        auth_token: &crate::ApiKey,
    ) -> anyhow::Result<Vec<crate::WorkspaceInfo>>;

    /// Get workspace information by workspace ID
    async fn get_workspace(
        &self,
        workspace_id: &WorkspaceId,
        auth_token: &crate::ApiKey,
    ) -> anyhow::Result<Option<crate::WorkspaceInfo>>;

    /// List all files in a workspace with their hashes
    async fn list_workspace_files(
        &self,
        workspace: &crate::WorkspaceFiles,
        auth_token: &crate::ApiKey,
    ) -> anyhow::Result<Vec<crate::FileHash>>;

    /// Delete files from a workspace
    async fn delete_files(
        &self,
        deletion: &crate::FileDeletion,
        auth_token: &crate::ApiKey,
    ) -> anyhow::Result<()>;

    /// Delete a workspace and all its indexed data
    async fn delete_workspace(
        &self,
        workspace_id: &WorkspaceId,
        auth_token: &crate::ApiKey,
    ) -> anyhow::Result<()>;
}

/// Repository for managing skills
///
/// This repository provides operations for loading and managing skills from
/// markdown files.
#[async_trait::async_trait]
pub trait SkillRepository: Send + Sync {
    /// Loads all available skills from the skills directory
    ///
    /// # Errors
    /// Returns an error if skill loading fails
    async fn load_skills(&self) -> Result<Vec<Skill>>;
}

/// Repository for validating file syntax
///
/// This repository provides operations for validating the syntax of source
/// code files using remote validation services.
#[async_trait::async_trait]
pub trait ValidationRepository: Send + Sync {
    /// Validates the syntax of a single file
    ///
    /// # Arguments
    /// * `path` - Path to the file (used for determining language and in error
    ///   messages)
    /// * `content` - Content of the file to validate
    ///
    /// # Returns
    /// * `Ok(vec![])` - File is valid or file type is not supported by backend
    /// * `Ok(errors)` - Validation failed with list of syntax errors
    /// * `Err(_)` - Communication error with validation service
    async fn validate_file(
        &self,
        path: impl AsRef<std::path::Path> + Send,
        content: &str,
    ) -> Result<Vec<crate::SyntaxError>>;
}

/// Repository for fuzzy searching text
///
/// This repository provides fuzzy search functionality for searching
/// needle in haystack with optional search_all flag.
#[async_trait::async_trait]
pub trait FuzzySearchRepository: Send + Sync {
    /// Performs a fuzzy search for a needle in a haystack
    ///
    /// # Arguments
    /// * `needle` - The string to search for
    /// * `haystack` - The text to search in
    /// * `search_all` - Whether to search all matches or just the first
    ///
    /// # Returns
    /// * `Ok(Vec<SearchMatch>)` - List of matches with line ranges
    /// * `Err(_)` - Communication error with search service
    async fn fuzzy_search(
        &self,
        needle: &str,
        haystack: &str,
        search_all: bool,
    ) -> Result<Vec<SearchMatch>>;
}

#[async_trait::async_trait]
pub trait TextPatchRepository: Send + Sync {
    async fn build_text_patch(
        &self,
        haystack: &str,
        old_string: &str,
        new_string: &str,
    ) -> Result<TextPatchBlock>;
}
