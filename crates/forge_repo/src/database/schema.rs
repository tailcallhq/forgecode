// @generated automatically by Diesel CLI.

diesel::table! {
    conversations (conversation_id) {
        conversation_id -> Text,
        title -> Nullable<Text>,
        workspace_id -> BigInt,
        context -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
        metrics -> Nullable<Text>,
        parent_id -> Nullable<Text>,
        source -> Nullable<Text>,
        #[sql_name = "cwd"]
        cwd -> Nullable<Text>,
        #[sql_name = "message_count"]
        message_count -> Nullable<Integer>,
        intent_state -> Text,
        extracted_at -> Nullable<Timestamp>,
        memory_id -> Nullable<Text>,
        intent_hash -> Nullable<Text>,
        context_zstd -> Nullable<Binary>,
        is_compressed -> Integer,
    }
}

// Read-only projection that UNIONs the local `conversations` table with
// a read-only legacy database ATTACHed at runtime by
// [`crate::database::pool::SqliteCustomizer`].
//
// `conversations_all` is created as a TEMP VIEW on every connection
// acquire. The columns mirror `conversations` 1:1, so Diesel queries
// against `conversations_all::table` return rows shaped like
// `ConversationRecord`.
//
// Only used for SELECT queries. INSERT/UPDATE/DELETE always target the
// local `conversations` table so the legacy DB is never mutated.
diesel::table! {
    conversations_all (conversation_id) {
        conversation_id -> Text,
        title -> Nullable<Text>,
        workspace_id -> BigInt,
        context -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
        metrics -> Nullable<Text>,
        parent_id -> Nullable<Text>,
        source -> Nullable<Text>,
        #[sql_name = "cwd"]
        cwd -> Nullable<Text>,
        #[sql_name = "message_count"]
        message_count -> Nullable<Integer>,
        intent_state -> Text,
        extracted_at -> Nullable<Timestamp>,
        memory_id -> Nullable<Text>,
        intent_hash -> Nullable<Text>,
        context_zstd -> Nullable<Binary>,
        is_compressed -> Integer,
    }
}
