use diesel::deserialize::{self, FromSql};
use diesel::serialize::{self, Output, ToSql};
use diesel::sql_types::Text;
use diesel::sqlite::Sqlite;
use diesel::{AsExpression, FromSqlRow};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Common API response structure for all endpoints
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ApiResponse<T: ToSchema> {
    /// Indicates whether the request was successful
    pub success: bool,
    /// Optional success message
    pub msg: Option<String>,
    /// Optional error message (present when success is false)
    pub error: Option<String>,
    /// Response data (present when success is true)
    pub data: Option<T>,
}

/// Empty data type for API responses without payload
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EmptyData;

impl<T: utoipa::ToSchema> ApiResponse<T> {
    /// Create a successful response with data
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            msg: None,
            error: None,
            data: Some(data),
        }
    }

    /// Create a successful response with data and message
    pub fn success_with_msg(data: T, msg: String) -> Self {
        Self {
            success: true,
            msg: Some(msg),
            error: None,
            data: Some(data),
        }
    }

    /// Create an error response
    pub fn error(error: String) -> Self {
        Self {
            success: false,
            msg: None,
            error: Some(error),
            data: None,
        }
    }

    /// Create an error response with custom message
    pub fn error_with_msg(error: String, msg: String) -> Self {
        Self {
            success: false,
            msg: Some(msg),
            error: Some(error),
            data: None,
        }
    }
}

impl ApiResponse<()> {
    /// Create a successful response without data (just success status)
    pub fn ok() -> Self {
        Self {
            success: true,
            msg: None,
            error: None,
            data: Some(()),
        }
    }

    /// Create a successful response without data but with a message
    pub fn ok_with_msg(msg: String) -> Self {
        Self {
            success: true,
            msg: Some(msg),
            error: None,
            data: Some(()),
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, AsExpression, FromSqlRow, ToSchema,
)]
#[diesel(sql_type = Text)]
pub enum PurchaseTokenStatus {
    /// Subscription token is pending acknowledgment with Google Play
    Pending,
    /// Subscription token acknowledged and service access successfully granted
    AccessGranted,
    /// Subscription token has expired or been canceled
    Expired,
}

impl ToSql<Text, Sqlite> for PurchaseTokenStatus {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Sqlite>) -> serialize::Result {
        match *self {
            PurchaseTokenStatus::AccessGranted => {
                <&str as ToSql<Text, Sqlite>>::to_sql(&"access_granted", out)
            }
            PurchaseTokenStatus::Expired => <&str as ToSql<Text, Sqlite>>::to_sql(&"expired", out),
            PurchaseTokenStatus::Pending => <&str as ToSql<Text, Sqlite>>::to_sql(&"pending", out),
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
            "access_granted" => Ok(PurchaseTokenStatus::AccessGranted),
            "expired" => Ok(PurchaseTokenStatus::Expired),
            _ => Err("Invalid purchase token status".into()),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, ToSchema)]
pub struct VerifyRequest {
    /// Unique identifier for the user
    pub user_id: String,
    /// Android package name
    pub package_name: String,
    /// Subscription ID from Google Play
    pub product_id: String,
    /// Subscription purchase token from Google Play
    pub purchase_token: String,
}

/// Empty response for verification endpoints
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct VerifyResponse {}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct AckRequest {
    /// Android package name
    pub package_name: String,
    /// Product ID from Google Play
    pub product_id: String,
    /// Purchase token from Google Play
    pub purchase_token: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AckData {
    /// Whether the acknowledgment was successful
    pub acknowledged: bool,
}

pub type AckResponse = ApiResponse<AckData>;

/// Simple response type for operations that don't return specific data
#[derive(Debug, Serialize, Deserialize)]
pub struct SimpleResponse {
    pub status: String,
}

pub type GenericResponse = ApiResponse<SimpleResponse>;

/// Common response type for operations that just need to indicate success/failure
pub type StatusResponse = ApiResponse<()>;

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

// Google Play Subscriptions v2 API response types
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GooglePlaySubscriptionResponse {
    pub kind: String,
    #[serde(rename = "startTime")]
    pub start_time: Option<String>,
    #[serde(rename = "regionCode")]
    pub region_code: Option<String>,
    #[serde(rename = "subscriptionState")]
    pub subscription_state: String,
    #[serde(rename = "latestOrderId")]
    pub latest_order_id: Option<String>,
    #[serde(rename = "acknowledgementState")]
    pub acknowledgement_state: String,
    #[serde(rename = "lineItems")]
    pub line_items: Vec<SubscriptionLineItem>,
    #[serde(rename = "linkedPurchaseToken")]
    pub linked_purchase_token: Option<String>,
    #[serde(rename = "purchaseToken")]
    pub purchase_token: String,
    #[serde(rename = "externalAccountIdentifiers")]
    pub external_account_identifiers: Option<ExternalAccountIdentifiers>,
    #[serde(rename = "subscribeWithGoogleInfo")]
    pub subscribe_with_google_info: Option<SubscribeWithGoogleInfo>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SubscriptionLineItem {
    #[serde(rename = "productId")]
    pub product_id: String,
    #[serde(rename = "expiryTime")]
    pub expiry_time: Option<String>,
    #[serde(rename = "autoRenewing")]
    pub auto_renewing: Option<bool>,
    #[serde(rename = "priceChangeState")]
    pub price_change_state: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema, Clone)]
pub struct ExternalAccountIdentifiers {
    #[serde(rename = "externalAccountId")]
    pub external_account_id: Option<String>,
    #[serde(rename = "obfuscatedExternalAccountId")]
    pub obfuscated_external_account_id: Option<String>,
    #[serde(rename = "obfuscatedExternalProfileId")]
    pub obfuscated_external_profile_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SubscribeWithGoogleInfo {
    #[serde(rename = "profileId")]
    pub profile_id: Option<String>,
    #[serde(rename = "profileName")]
    pub profile_name: Option<String>,
    #[serde(rename = "emailAddress")]
    pub email_address: Option<String>,
    #[serde(rename = "givenName")]
    pub given_name: Option<String>,
    #[serde(rename = "familyName")]
    pub family_name: Option<String>,
}

// Google Play subscription states
pub mod google_play_subscription_state {
    pub const SUBSCRIPTION_STATE_UNSPECIFIED: &str = "SUBSCRIPTION_STATE_UNSPECIFIED";
    pub const SUBSCRIPTION_STATE_PENDING: &str = "SUBSCRIPTION_STATE_PENDING";
    pub const SUBSCRIPTION_STATE_ACTIVE: &str = "SUBSCRIPTION_STATE_ACTIVE";
    pub const SUBSCRIPTION_STATE_PAUSED: &str = "SUBSCRIPTION_STATE_PAUSED";
    pub const SUBSCRIPTION_STATE_IN_GRACE_PERIOD: &str = "SUBSCRIPTION_STATE_IN_GRACE_PERIOD";
    pub const SUBSCRIPTION_STATE_ON_HOLD: &str = "SUBSCRIPTION_STATE_ON_HOLD";
    pub const SUBSCRIPTION_STATE_CANCELED: &str = "SUBSCRIPTION_STATE_CANCELED";
    pub const SUBSCRIPTION_STATE_EXPIRED: &str = "SUBSCRIPTION_STATE_EXPIRED";
}

// Google Play acknowledgement states
pub mod google_play_acknowledgement_state {
    pub const ACKNOWLEDGEMENT_STATE_UNSPECIFIED: &str = "ACKNOWLEDGEMENT_STATE_UNSPECIFIED";
    pub const ACKNOWLEDGEMENT_STATE_PENDING: &str = "ACKNOWLEDGEMENT_STATE_PENDING";
    pub const ACKNOWLEDGEMENT_STATE_ACKNOWLEDGED: &str = "ACKNOWLEDGEMENT_STATE_ACKNOWLEDGED";
}
