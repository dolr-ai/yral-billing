// @generated automatically by Diesel CLI.

diesel::table! {
    apple_app_account_tokens (id) {
        id -> Text,
        app_account_token -> Text,
        user_id -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    bot_chat_access (id) {
        id -> Text,
        purchase_source -> Text,
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
    bot_subscriptions (id) {
        id -> Text,
        purchase_source -> Text,
        purchase_token -> Text,
        user_id -> Text,
        bot_id -> Text,
        product_id -> Text,
        status -> Text,
        started_at -> Timestamp,
        updated_at -> Timestamp,
        expires_at -> Timestamp,
    }
}

diesel::table! {
    image_access (id) {
        id -> Text,
        purchase_source -> Text,
        purchase_token -> Text,
        user_id -> Text,
        bot_id -> Text,
        image_id -> Text,
        status -> Text,
        granted_at -> Timestamp,
        updated_at -> Timestamp,
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

diesel::table! {
    transactions (id) {
        id -> Text,
        user_id -> Text,
        transaction_type -> Text,
        amount_paise -> BigInt,
        recipient_id -> Text,
        purchase_source -> Text,
        purchase_token -> Text,
        created_at -> Timestamp,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    apple_app_account_tokens,
    bot_chat_access,
    bot_subscriptions,
    image_access,
    purchase_tokens,
    transactions,
);
