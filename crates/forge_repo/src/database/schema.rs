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
    }
}

diesel::table! {
    snapshot_metadata (snapshot_id) {
        snapshot_id -> Text,
        user_input_id -> Text,
        conversation_id -> Text,
        file_path -> Text,
        snap_file_path -> Text,
        created_at -> Timestamp,
    }
}
