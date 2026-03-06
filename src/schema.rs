// @generated automatically by Diesel CLI.

diesel::table! {
    bot_chat_access (id) {
        id -> Text,
        purchase_token -> Text,
        user_id -> Text,
        bot_id -> Text,
        status -> Text,
        granted_at -> Timestamp,
        updated_at -> Timestamp,
        expires_at -> Timestamp,
    }
}

diesel::table! {
    purchase_tokens (id) {
        id -> Text,
        user_id -> Text,
        purchase_token -> Text,
        status -> Text,
        created_at -> Timestamp,
        expiry_at -> Timestamp,
    }
}

diesel::allow_tables_to_appear_in_same_query!(bot_chat_access, purchase_tokens,);
