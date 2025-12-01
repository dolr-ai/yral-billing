use diesel::deserialize::{self, FromSql};
use diesel::serialize::{self, Output, ToSql};
use diesel::sql_types::Text;
use diesel::sqlite::Sqlite;
use diesel::{AsExpression, FromSqlRow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, AsExpression, FromSqlRow)]
#[diesel(sql_type = Text)]
pub enum PurchaseTokenStatus {
    Pending,
    Acknowledged,
    Expired,
}

impl ToSql<Text, Sqlite> for PurchaseTokenStatus {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Sqlite>) -> serialize::Result {
        match *self {
            PurchaseTokenStatus::Pending => <&str as ToSql<Text, Sqlite>>::to_sql(&"pending", out),
            PurchaseTokenStatus::Acknowledged => {
                <&str as ToSql<Text, Sqlite>>::to_sql(&"acknowledged", out)
            }
            PurchaseTokenStatus::Expired => <&str as ToSql<Text, Sqlite>>::to_sql(&"expired", out),
        }
    }
}

impl FromSql<Text, Sqlite> for PurchaseTokenStatus {
    fn from_sql(
        bytes: <Sqlite as diesel::backend::Backend>::RawValue<'_>,
    ) -> deserialize::Result<Self> {
        let status_str = <String as FromSql<Text, Sqlite>>::from_sql(bytes)?;
        match status_str.as_str() {
            "pending" => Ok(PurchaseTokenStatus::Pending),
            "acknowledged" => Ok(PurchaseTokenStatus::Acknowledged),
            "expired" => Ok(PurchaseTokenStatus::Expired),
            _ => Err("Invalid purchase token status".into()),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct VerifyRequest {
    pub user_id: String,
    pub package_name: String,
    pub product_id: String,
    pub purchase_token: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyResponse {
    pub acknowledged: bool,
    pub status: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct AckRequest {
    pub package_name: String,
    pub product_id: String,
    pub purchase_token: String,
}

#[derive(Debug, Serialize)]
pub struct AckResponse {
    pub success: bool,
    pub status: String,
}

// RTDN (Real-time Developer Notifications) Types
#[derive(Debug, Deserialize, Serialize)]
pub struct DeveloperNotification {
    pub version: String,
    #[serde(rename = "packageName")]
    pub package_name: String,
    #[serde(rename = "eventTimeMillis")]
    pub event_time_millis: String,
    #[serde(rename = "subscriptionNotification")]
    pub subscription_notification: Option<SubscriptionNotification>,
    #[serde(rename = "oneTimeProductNotification")]
    pub one_time_product_notification: Option<OneTimeProductNotification>,
    #[serde(rename = "testNotification")]
    pub test_notification: Option<TestNotification>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SubscriptionNotification {
    pub version: String,
    #[serde(rename = "notificationType")]
    pub notification_type: i32,
    #[serde(rename = "purchaseToken")]
    pub purchase_token: String,
    #[serde(rename = "subscriptionId")]
    pub subscription_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct OneTimeProductNotification {
    pub version: String,
    #[serde(rename = "notificationType")]
    pub notification_type: i32,
    #[serde(rename = "purchaseToken")]
    pub purchase_token: String,
    pub sku: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TestNotification {
    pub version: String,
}

#[derive(Debug, Deserialize)]
pub struct PubSubMessage {
    pub message: PubSubData,
}

#[derive(Debug, Deserialize)]
pub struct PubSubData {
    pub data: String,
    #[serde(rename = "messageId")]
    pub message_id: String,
    #[serde(rename = "publishTime")]
    pub publish_time: String,
}

// Notification types for subscriptions
pub mod subscription_notification_type {
    pub const SUBSCRIPTION_RECOVERED: i32 = 1;
    pub const SUBSCRIPTION_RENEWED: i32 = 2;
    pub const SUBSCRIPTION_CANCELED: i32 = 3;
    pub const SUBSCRIPTION_PURCHASED: i32 = 4;
    pub const SUBSCRIPTION_ON_HOLD: i32 = 5;
    pub const SUBSCRIPTION_IN_GRACE_PERIOD: i32 = 6;
    pub const SUBSCRIPTION_RESTARTED: i32 = 7;
    pub const SUBSCRIPTION_PRICE_CHANGE_CONFIRMED: i32 = 8;
    pub const SUBSCRIPTION_DEFERRED: i32 = 9;
    pub const SUBSCRIPTION_PAUSED: i32 = 10;
    pub const SUBSCRIPTION_PAUSE_SCHEDULE_CHANGED: i32 = 11;
    pub const SUBSCRIPTION_REVOKED: i32 = 12;
    pub const SUBSCRIPTION_EXPIRED: i32 = 13;
}

// Notification types for one-time products
pub mod one_time_product_notification_type {
    pub const ONE_TIME_PRODUCT_PURCHASED: i32 = 1;
    pub const ONE_TIME_PRODUCT_CANCELED: i32 = 2;
}
