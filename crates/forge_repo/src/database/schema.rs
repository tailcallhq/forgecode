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
    }
}
