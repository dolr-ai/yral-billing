use crate::types::ApiResponse;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};

/// Application-specific error types
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Database connection failed")]
    DatabaseConnection,

    #[error("Database operation failed: {0}")]
    DatabaseOperation(String),

    #[error("Google Play API error: {0}")]
    GooglePlayApi(String),

    #[error("Google Play verification failed: {0}")]
    GooglePlayVerification(String),

    #[error("Authentication service unavailable")]
    AuthServiceUnavailable,

    #[error("Admin IC agent is missing")]
    AdminIcAgentMissing,

    #[error("Failed to get access token: {0}")]
    AccessTokenFailed(String),

    #[error("Purchase token already used by different user")]
    TokenAlreadyUsed,

    #[error("Purchase token has expired")]
    TokenExpired,

    #[error("Subscription has been canceled")]
    SubscriptionCanceled,

    #[error("Subscription has expired")]
    SubscriptionExpired,

    #[error("Subscription is on hold")]
    SubscriptionOnHold,

    #[error("Subscription is paused by user")]
    SubscriptionPaused,

    #[error("Subscription is active but has no valid line items")]
    SubscriptionInvalidLineItems,

    #[error("Unknown or invalid subscription state")]
    SubscriptionInvalidState,

    #[error("No subscription state found in response")]
    SubscriptionNoState,

    #[error("Failed to parse Google Play response: {0}")]
    GooglePlayResponseParse(String),

    #[error("Failed to connect to Google Play API: {0}")]
    GooglePlayConnection(String),

    #[error("Failed to acknowledge purchase with Google Play")]
    AcknowledgmentFailed,

    #[error("Failed to grant service access: {0}")]
    ServiceAccessFailed(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Internal server error: {0}")]
    InternalError(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("External account identifiers are missing")]
    ExternalAccountIdentifiersMissing,
}

impl AppError {
    /// Get the appropriate HTTP status code for this error
    fn status_code(&self) -> StatusCode {
        match self {
            AppError::DatabaseConnection
            | AppError::DatabaseOperation(_)
            | AppError::AuthServiceUnavailable
            | AppError::AdminIcAgentMissing
            | AppError::AccessTokenFailed(_)
            | AppError::ServiceAccessFailed(_)
            | AppError::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,

            AppError::GooglePlayApi(_)
            | AppError::GooglePlayVerification(_)
            | AppError::TokenAlreadyUsed
            | AppError::TokenExpired
            | AppError::SubscriptionCanceled
            | AppError::SubscriptionExpired
            | AppError::SubscriptionInvalidLineItems
            | AppError::SubscriptionInvalidState
            | AppError::SubscriptionNoState
            | AppError::GooglePlayResponseParse(_)
            | AppError::AcknowledgmentFailed
            | AppError::ExternalAccountIdentifiersMissing
            | AppError::BadRequest(_) => StatusCode::BAD_REQUEST,

            AppError::SubscriptionOnHold | AppError::SubscriptionPaused => StatusCode::ACCEPTED, // 202 - acknowledged but not processed

            AppError::GooglePlayConnection(_) | AppError::NetworkError(_) => {
                StatusCode::BAD_GATEWAY
            }
        }
    }

    /// Get the error message
    fn message(&self) -> String {
        self.to_string()
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let status_code = self.status_code();
        let error_message = self.message();

        let response_body = ApiResponse::<()>::error(error_message);

        (status_code, Json(response_body)).into_response()
    }
}

/// Result type for application operations
pub type AppResult<T> = Result<T, AppError>;

// Conversion implementations for common error types
impl From<diesel::result::Error> for AppError {
    fn from(err: diesel::result::Error) -> Self {
        AppError::DatabaseOperation(err.to_string())
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_connect() || err.is_timeout() {
            AppError::NetworkError(err.to_string())
        } else {
            AppError::GooglePlayConnection(err.to_string())
        }
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for AppError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        AppError::InternalError(err.to_string())
    }
}

impl From<String> for AppError {
    fn from(err: String) -> Self {
        AppError::InternalError(err)
    }
}

impl From<&str> for AppError {
    fn from(err: &str) -> Self {
        AppError::InternalError(err.to_string())
    }
}
