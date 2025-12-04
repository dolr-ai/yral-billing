use crate::types::PurchaseTokenStatus;
use chrono::NaiveDateTime;
use diesel::prelude::*;
use uuid::Uuid;

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
