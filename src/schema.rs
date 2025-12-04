// @generated automatically by Diesel CLI.

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
