use crate::types::{BotChatAccessStatus, PurchaseTokenStatus};
use chrono::NaiveDateTime;
use diesel::prelude::*;
use uuid::Uuid;

#[derive(Queryable, Insertable, Identifiable, Debug, Clone)]
#[diesel(table_name = crate::schema::bot_chat_access)]
pub struct BotChatAccess {
    pub id: String,
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
        purchase_token: String,
        user_id: String,
        bot_id: String,
        expires_at: NaiveDateTime,
    ) -> Self {
        let now = chrono::Utc::now().naive_utc();
        Self {
            id: Uuid::new_v4().to_string(),
            purchase_token,
            user_id,
            bot_id,
            status: BotChatAccessStatus::Active,
            granted_at: now,
            updated_at: now,
            expires_at,
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
