use crate::types::PurchaseTokenStatus;
use chrono::NaiveDateTime;
use diesel::prelude::*;
use uuid::Uuid;

#[derive(Queryable, Identifiable, Debug, Clone)]
#[diesel(table_name = crate::schema::purchase_tokens)]
pub struct PurchaseToken {
    pub id: String,
    pub user_id: String,
    pub purchase_token: String,
    pub status: PurchaseTokenStatus,
    pub created_at: NaiveDateTime,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::purchase_tokens)]
pub struct NewPurchaseToken {
    pub id: Option<String>,
    pub user_id: String,
    pub purchase_token: String,
    pub status: Option<PurchaseTokenStatus>,
    pub created_at: NaiveDateTime,
}

impl NewPurchaseToken {
    pub fn new(user_id: String, purchase_token: String) -> Self {
        Self {
            id: Some(Uuid::new_v4().to_string()),
            user_id,
            purchase_token,
            status: Some(PurchaseTokenStatus::Pending),
            created_at: chrono::Utc::now().naive_utc(),
        }
    }
}
