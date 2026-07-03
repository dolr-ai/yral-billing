use crate::types::{BotChatAccessStatus, PurchaseSource, PurchaseTokenStatus, TransactionType};
use chrono::NaiveDateTime;
use diesel::prelude::*;
use uuid::Uuid;

#[derive(Queryable, Insertable, Identifiable, Debug, Clone)]
#[diesel(table_name = crate::schema::bot_chat_access)]
pub struct BotChatAccess {
    pub id: String,
    pub purchase_source: PurchaseSource,
    pub purchase_token: String,
    pub user_id: String,
    pub bot_id: String,
    pub status: BotChatAccessStatus,
    pub granted_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
}

impl BotChatAccess {
    pub fn new(
        purchase_source: PurchaseSource,
        purchase_token: String,
        user_id: String,
        bot_id: String,
        expires_at: NaiveDateTime,
    ) -> Self {
        let now = chrono::Utc::now().naive_utc();
        Self {
            id: Uuid::new_v4().to_string(),
            purchase_source,
            purchase_token,
            user_id,
            bot_id,
            status: BotChatAccessStatus::ConsumePending,
            granted_at: now,
            updated_at: now,
            expires_at,
        }
    }
}

/// Per-image unlock purchased with the `image_unlock` consumable.
/// Access is permanent (no expiry); `BotChatAccessStatus::Expired` is never
/// written for these rows.
#[derive(Queryable, Insertable, Identifiable, Debug, Clone)]
#[diesel(table_name = crate::schema::image_access)]
pub struct ImageAccess {
    pub id: String,
    pub purchase_source: PurchaseSource,
    pub purchase_token: String,
    pub user_id: String,
    pub bot_id: String,
    pub image_id: String,
    pub status: BotChatAccessStatus,
    pub granted_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl ImageAccess {
    pub fn new(
        purchase_source: PurchaseSource,
        purchase_token: String,
        user_id: String,
        bot_id: String,
        image_id: String,
    ) -> Self {
        let now = chrono::Utc::now().naive_utc();
        Self {
            id: Uuid::new_v4().to_string(),
            purchase_source,
            purchase_token,
            user_id,
            bot_id,
            image_id,
            status: BotChatAccessStatus::ConsumePending,
            granted_at: now,
            updated_at: now,
        }
    }
}

#[derive(Queryable, Insertable, Identifiable, Debug, Clone)]
#[diesel(table_name = crate::schema::purchase_tokens)]
pub struct PurchaseToken {
    pub id: String,
    pub user_id: String,
    pub purchase_token: String,
    pub status: PurchaseTokenStatus,
    pub created_at: NaiveDateTime,
    pub expiry_at: NaiveDateTime,
}

impl PurchaseToken {
    pub fn new(
        user_id: String,
        purchase_token: String,
        expiry_at: NaiveDateTime,
        status: PurchaseTokenStatus,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            user_id,
            purchase_token,
            status,
            created_at: chrono::Utc::now().naive_utc(),
            expiry_at,
        }
    }
}

#[derive(Queryable, Insertable, Identifiable, Debug, Clone)]
#[diesel(table_name = crate::schema::transactions)]
pub struct Transaction {
    pub id: String,
    pub user_id: String,
    pub transaction_type: TransactionType,
    pub amount_paise: i64,
    pub recipient_id: String,
    pub purchase_source: PurchaseSource,
    pub purchase_token: String,
    pub created_at: NaiveDateTime,
}

impl Transaction {
    pub fn new(
        user_id: String,
        transaction_type: TransactionType,
        amount_paise: i64,
        recipient_id: String,
        purchase_source: PurchaseSource,
        purchase_token: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            user_id,
            transaction_type,
            amount_paise,
            recipient_id,
            purchase_source,
            purchase_token,
            created_at: chrono::Utc::now().naive_utc(),
        }
    }
}

#[derive(Queryable, Insertable, Identifiable, Debug, Clone)]
#[diesel(table_name = crate::schema::apple_app_account_tokens)]
pub struct AppleAppAccountToken {
    pub id: String,
    pub app_account_token: String,
    pub user_id: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl AppleAppAccountToken {
    pub fn new(user_id: String) -> Self {
        let now = chrono::Utc::now().naive_utc();
        Self {
            id: Uuid::new_v4().to_string(),
            app_account_token: Uuid::new_v4().to_string(),
            user_id,
            created_at: now,
            updated_at: now,
        }
    }
}
