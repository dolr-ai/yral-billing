use crate::auth::GoogleAuth;
use crate::model::{NewPurchaseToken, PurchaseToken};
use crate::types::{PurchaseTokenStatus, VerifyRequest, VerifyResponse};
use crate::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::{response::IntoResponse, Json};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use std::env;
use std::sync::Arc;

async fn verify_with_google_play(
    payload: &VerifyRequest,
    auth: Option<&Arc<GoogleAuth>>,
) -> Result<serde_json::Value, StatusCode> {
    // Use mock verification when local or mock-google-api feature is enabled
    #[cfg(any(feature = "local", feature = "mock-google-api"))]
    {
        let _ = payload; // Suppress unused variable warning
                         // Return mock successful verification for local development and tests
        return Ok(serde_json::json!({
            "acknowledgementState": 0,
            "purchaseState": 1,
            "consumptionState": 1,
            "developerPayload": "",
            "purchaseTimeMillis": "1234567890123",
            "purchaseState": 1
        }));
    }

    #[cfg(not(any(feature = "local", feature = "mock-google-api")))]
    {
        // Get OAuth access token from app state
        let auth = auth.ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        let access_token = auth
            .get_token_for_default_scopes()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let url = format!(
            "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/purchases/products/{}/tokens/{}",
            payload.package_name, payload.product_id, payload.purchase_token
        );

        let client = reqwest::Client::new();
        let res = client.get(&url).bearer_auth(&access_token).send().await;

        match res {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<serde_json::Value>().await {
                        Ok(json) => Ok(json),
                        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
                    }
                } else {
                    Err(response.status())
                }
            }
            Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }
}

async fn acknowledge_google_play(
    payload: &VerifyRequest,
    auth: Option<&Arc<GoogleAuth>>,
) -> Result<(), StatusCode> {
    // Use mock acknowledgment when local or mock-google-api feature is enabled
    #[cfg(any(feature = "local", feature = "mock-google-api"))]
    {
        let _ = payload; // Suppress unused variable warning
                         // Mock successful acknowledgment for local development and tests
        return Ok(());
    }

    #[cfg(not(any(feature = "local", feature = "mock-google-api")))]
    {
        // Get OAuth access token from app state
        let auth = auth.ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        let access_token = auth
            .get_token_for_default_scopes()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let ack_url = format!(
            "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/purchases/products/{}/tokens/{}/:acknowledge",
            payload.package_name, payload.product_id, payload.purchase_token
        );

        let client = reqwest::Client::new();
        let ack_res = client
            .post(&ack_url)
            .bearer_auth(&access_token)
            .send()
            .await;

        match ack_res {
            Ok(r) if r.status().is_success() => Ok(()),
            Ok(r) => Err(r.status()),
            Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }
}

pub async fn verify_purchase(
    State(app_state): State<AppState>,
    Json(payload): Json<VerifyRequest>,
) -> impl IntoResponse {
    use crate::schema::purchase_tokens::dsl::*;

    // Use test database for tests, production database otherwise
    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| "billing.db".to_string());
    let mut conn = match SqliteConnection::establish(&database_url) {
        Ok(conn) => conn,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(VerifyResponse {
                    acknowledged: false,
                    status: "Database connection error".to_string(),
                }),
            );
        }
    };

    // Check if this purchase token already exists
    let existing_token: Option<PurchaseToken> = purchase_tokens
        .filter(purchase_token.eq(&payload.purchase_token))
        .first(&mut conn)
        .optional()
        .unwrap_or(None);

    match existing_token {
        Some(token) => {
            if token.user_id == payload.user_id {
                // Same user, same token - return success (idempotent)
                (
                    StatusCode::OK,
                    Json(VerifyResponse {
                        acknowledged: true,
                        status: "Token already verified for this user".to_string(),
                    }),
                )
            } else {
                // Different user trying to use same token
                (
                    StatusCode::BAD_REQUEST,
                    Json(VerifyResponse {
                        acknowledged: false,
                        status: "Purchase token already used by different user".to_string(),
                    }),
                )
            }
        }
        None => {
            // First verify with Google Play API
            let google_verification =
                verify_with_google_play(&payload, app_state.google_auth.as_ref()).await;

            match google_verification {
                Ok(json) => {
                    // Purchase is valid with Google, now store in database
                    let mut new_token = NewPurchaseToken::new(
                        payload.user_id.clone(),
                        payload.purchase_token.clone(),
                    );

                    // Check if already acknowledged
                    let already_acknowledged =
                        json.get("acknowledgementState").and_then(|v| v.as_i64()) == Some(1);

                    if !already_acknowledged {
                        // Need to acknowledge with Google
                        match acknowledge_google_play(&payload, app_state.google_auth.as_ref())
                            .await
                        {
                            Ok(_) => {
                                new_token.status = Some(PurchaseTokenStatus::Acknowledged);
                            }
                            Err(_) => {
                                new_token.status = Some(PurchaseTokenStatus::Pending);
                            }
                        }
                    } else {
                        new_token.status = Some(PurchaseTokenStatus::Acknowledged);
                    }

                    // Store in database
                    match diesel::insert_into(purchase_tokens)
                        .values(&new_token)
                        .execute(&mut conn)
                    {
                        Ok(_) => (
                            StatusCode::OK,
                            Json(VerifyResponse {
                                acknowledged: new_token.status
                                    == Some(PurchaseTokenStatus::Acknowledged),
                                status: "Purchase verified and recorded".to_string(),
                            }),
                        ),
                        Err(_) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(VerifyResponse {
                                acknowledged: false,
                                status: "Failed to record purchase".to_string(),
                            }),
                        ),
                    }
                }
                Err(status_code) => {
                    // Google Play verification failed
                    (
                        status_code,
                        Json(VerifyResponse {
                            acknowledged: false,
                            status: format!("Google Play verification failed: {}", status_code),
                        }),
                    )
                }
            }
        }
    }
}
