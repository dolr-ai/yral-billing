use std::sync::Arc;

use crate::auth::{GoogleAuth, GooglePublicKey};
use crate::error::AppError;
use crate::model::PurchaseToken;
use crate::routes::goole_play_billing_helpers::{
    acknowledge_google_play, fetch_google_play_purchase_details,
};
use crate::routes::purchase_token_helpers::verify_subcription_response_for_active_status;
use crate::routes::utils::{grant_yral_pro_plan_access, revoke_yral_pro_plan_access};
use crate::types::{
    subscription_notification_type, DeveloperNotification, GooglePlaySubscriptionResponse,
    PubSubMessage, PurchaseTokenStatus,
};
use axum::http::HeaderMap;
use axum::{http::StatusCode, response::IntoResponse, Json};
use base64::prelude::*;
use diesel::r2d2::{ConnectionManager, PooledConnection};
use diesel::{prelude::*, RunQueryDsl};
use reqwest::header::AUTHORIZATION;
use serde_json;

pub async fn verify_rtdn_webhook(
    header_value: Option<&axum::http::HeaderValue>,
    google_public_key: Arc<GooglePublicKey>,
) -> Result<(), Box<dyn std::error::Error>> {
    let auth_header = header_value.ok_or("Missing Authorization header")?;
    let auth_token = auth_header.to_str()?.trim_start_matches("Bearer ").trim();

    google_public_key.validate_token(auth_token).await?;

    Ok(())
}

pub async fn handle_rtdn_webhook(
    header_map: HeaderMap,
    axum::extract::State(app_state): axum::extract::State<crate::AppState>,
    Json(payload): Json<PubSubMessage>,
) -> impl IntoResponse {
    println!("Received RTDN webhook: {:?}", payload);

    let auth_header = header_map.get(AUTHORIZATION).take();

    if let Err(e) = verify_rtdn_webhook(auth_header, app_state.google_public_key.clone()).await {
        eprintln!("Authentication failed: {}", e);
        return (StatusCode::UNAUTHORIZED, "Unauthorized");
    }

    // Decode the base64 message data
    let decoded_data = match BASE64_STANDARD.decode(&payload.message.data) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Failed to decode base64 data: {}", e);
            return (StatusCode::BAD_REQUEST, "Invalid base64 data");
        }
    };

    // Parse the developer notification
    let notification_json = match String::from_utf8(decoded_data) {
        Ok(json_str) => json_str,
        Err(e) => {
            eprintln!("Failed to convert to UTF-8: {}", e);
            return (StatusCode::BAD_REQUEST, "Invalid UTF-8 data");
        }
    };

    let notification: DeveloperNotification = match serde_json::from_str(&notification_json) {
        Ok(notif) => notif,
        Err(e) => {
            eprintln!("Failed to parse notification JSON: {}", e);
            return (StatusCode::BAD_REQUEST, "Invalid notification format");
        }
    };

    // Process the notification
    match process_notification(&notification, &app_state).await {
        Ok(_) => {
            println!(
                "Successfully processed notification for package: {}",
                notification.package_name
            );
            // HTTP 200 acknowledges the message to Pub/Sub - Google requires simple success response
            (StatusCode::OK, "OK")
        }
        Err(e) => {
            eprintln!("Failed to process notification: {}", e);
            // HTTP 500 causes Pub/Sub to retry delivery
            // Consider returning 200 for permanent failures to avoid infinite retries
            (StatusCode::INTERNAL_SERVER_ERROR, "Processing failed")
        }
    }
}

async fn process_notification(
    notification: &DeveloperNotification,
    app_state: &crate::AppState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!(
        "Processing notification for package: {}",
        notification.package_name
    );
    println!("Event time: {}", notification.event_time_millis);

    // Handle subscription notifications
    if let Some(sub_notification) = &notification.subscription_notification {
        handle_subscription_notification(sub_notification, app_state, &notification.package_name)
            .await?;
    }

    // Handle one-time product notifications

    // Handle test notifications
    if let Some(test_notification) = &notification.test_notification {
        handle_test_notification(test_notification).await?;
    }

    Ok(())
}

pub async fn handle_new_subscription_purchase(
    conn: &mut PooledConnection<ConnectionManager<SqliteConnection>>,
    auth: Option<&Arc<GoogleAuth>>,
    admin_ic_agent: &ic_agent::Agent,
    package_name: &str,
    user_id_str: &str,
    purchase_token_param: &str,
    subscription_response: &GooglePlaySubscriptionResponse,
) -> Result<(), AppError> {
    use crate::schema::purchase_tokens::dsl::*;

    // Check if this purchase token already exists
    let existing_token: Option<PurchaseToken> = purchase_tokens
        .filter(purchase_token.eq(purchase_token_param))
        .first(conn)
        .optional()?;

    let expiry = subscription_response
        .line_items
        .iter()
        .find(|item| item.product_id == subscription_response.line_items[0].product_id)
        .map(|item| item.expiry_time.clone())
        .ok_or(AppError::SubscriptionInvalidLineItems)?;

    match existing_token {
        Some(token) => {
            // Update existing token with new expiry and status
            let expiry_native = expiry
                .and_then(|time_str| chrono::DateTime::parse_from_rfc3339(&time_str).ok())
                .map(|dt| dt.naive_utc())
                .ok_or(AppError::SubscriptionInvalidLineItems)?;

            diesel::update(purchase_tokens.filter(id.eq(&token.id)))
                .set((
                    expiry_at.eq(expiry_native),
                    status.eq(PurchaseTokenStatus::AccessGranted),
                ))
                .execute(conn)?;

            Ok(())
        }
        None => {
            verify_subcription_response_for_active_status(subscription_response)?;
            acknowledge_google_play(
                package_name,
                purchase_token_param,
                subscription_response,
                auth,
            )
            .await?;
            grant_yral_pro_plan_access(admin_ic_agent, user_id_str).await?;

            // Insert new purchase token into database
            let expiry_native = expiry
                .and_then(|time_str| chrono::DateTime::parse_from_rfc3339(&time_str).ok())
                .map(|dt| dt.naive_utc())
                .ok_or(AppError::SubscriptionInvalidLineItems)?;

            let new_token = PurchaseToken::new(
                user_id_str.to_string(),
                purchase_token_param.to_string(),
                expiry_native,
                PurchaseTokenStatus::AccessGranted,
            );

            diesel::insert_into(purchase_tokens)
                .values(&new_token)
                .execute(conn)?;

            Ok(())
        }
    }
}

async fn handle_subscription_renewal(
    conn: &mut PooledConnection<ConnectionManager<SqliteConnection>>,
    admin_ic_agent: &ic_agent::Agent,
    user_id_param: &str,
    purchase_token_param: &str,
    subscription_response: &GooglePlaySubscriptionResponse,
) -> Result<(), AppError> {
    use crate::schema::purchase_tokens::dsl::*;

    // Check if this purchase token already exists
    let existing_token: Option<PurchaseToken> = purchase_tokens
        .filter(purchase_token.eq(purchase_token_param))
        .first(conn)
        .optional()?;

    let expiry = subscription_response
        .line_items
        .iter()
        .find(|item| item.product_id == subscription_response.line_items[0].product_id)
        .map(|item| item.expiry_time.clone())
        .ok_or(AppError::SubscriptionInvalidLineItems)?;

    match existing_token {
        Some(token) => {
            // Update existing token with new expiry and status
            let expiry_native = expiry
                .and_then(|time_str| chrono::DateTime::parse_from_rfc3339(&time_str).ok())
                .map(|dt| dt.naive_utc())
                .ok_or(AppError::SubscriptionInvalidLineItems)?;

            grant_yral_pro_plan_access(admin_ic_agent, user_id_param).await?;

            diesel::update(purchase_tokens.filter(id.eq(&token.id)))
                .set((
                    expiry_at.eq(expiry_native),
                    status.eq(PurchaseTokenStatus::AccessGranted),
                ))
                .execute(conn)?;

            Ok(())
        }
        None => Err(AppError::SubscriptionInvalidLineItems),
    }
}

async fn handle_revoking_user_access(
    conn: &mut PooledConnection<ConnectionManager<SqliteConnection>>,
    admin_ic_agent: &ic_agent::Agent,
    user_id_str: &str,
    purchase_token_param: &str,
    _subscription_response: &GooglePlaySubscriptionResponse,
) -> Result<(), AppError> {
    use crate::schema::purchase_tokens::dsl::*;

    // Check if this purchase token already exists
    let existing_token: Option<PurchaseToken> = purchase_tokens
        .filter(purchase_token.eq(purchase_token_param))
        .first(conn)
        .optional()?;

    match existing_token {
        Some(token) => {
            // Update existing token with new expiry and status

            revoke_yral_pro_plan_access(admin_ic_agent, user_id_str).await?;

            diesel::update(purchase_tokens.filter(id.eq(&token.id)))
                .set((status.eq(PurchaseTokenStatus::Expired),))
                .execute(conn)?;

            Ok(())
        }
        None => {
            return Err(AppError::SubscriptionInvalidLineItems);
        }
    }
}

async fn handle_subscription_notification(
    notification: &crate::types::SubscriptionNotification,
    app_state: &crate::AppState,
    package_name: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let notification_type = notification.notification_type;
    let purchase_token = &notification.purchase_token;
    let subscription_id = &notification.subscription_id;

    println!(
        "Subscription notification - Type: {}, Token: {}, ID: {}",
        notification_type, purchase_token, subscription_id
    );

    // Get user ID from purchase details using obfuscatedAccountId set by client
    let google_play_subscription_response = fetch_google_play_purchase_details(
        package_name,
        &purchase_token,
        app_state.google_auth.as_ref(),
    )
    .await?;

    let user_id = google_play_subscription_response
        .external_account_identifiers
        .clone()
        .ok_or(AppError::ExternalAccountIdentifiersMissing)?
        .obfuscated_external_account_id
        .ok_or(AppError::ExternalAccountIdentifiersMissing)?;

    println!("Processing subscription notification for user: {}", user_id);

    handle_linked_purchase_token(
        &mut app_state.get_db_connection()?,
        google_play_subscription_response
            .linked_purchase_token
            .clone(),
    )?;

    match notification_type {
        subscription_notification_type::SUBSCRIPTION_PURCHASED => {
            handle_new_subscription_purchase(
                &mut app_state
                    .get_db_connection()
                    .map_err(|_| AppError::DatabaseConnection)?,
                app_state.google_auth.as_ref(),
                app_state
                    .admin_ic_agent
                    .as_ref()
                    .ok_or(AppError::AdminIcAgentMissing)?,
                package_name,
                &user_id,
                purchase_token,
                &google_play_subscription_response,
            )
            .await?;
        }
        subscription_notification_type::SUBSCRIPTION_RENEWED => {
            handle_subscription_renewal(
                &mut app_state
                    .get_db_connection()
                    .map_err(|_| AppError::DatabaseConnection)?,
                app_state
                    .admin_ic_agent
                    .as_ref()
                    .ok_or(AppError::AdminIcAgentMissing)?,
                &user_id,
                purchase_token,
                &google_play_subscription_response,
            )
            .await?;
        }
        subscription_notification_type::SUBSCRIPTION_CANCELED => {
            println!("Subscription canceled for user: {}", user_id);
            // we don't need to anything as we will expire the subscriptino on expiry
        }

        subscription_notification_type::SUBSCRIPTION_RECOVERED => {
            // in case of recovered we need to grant access again and update the expiry the token was expired
            handle_subscription_renewal(
                &mut app_state
                    .get_db_connection()
                    .map_err(|_| AppError::DatabaseConnection)?,
                app_state
                    .admin_ic_agent
                    .as_ref()
                    .ok_or(AppError::AdminIcAgentMissing)?,
                &user_id,
                purchase_token,
                &google_play_subscription_response,
            )
            .await?;
        }
        subscription_notification_type::SUBSCRIPTION_IN_GRACE_PERIOD => {
            println!("Subscription in grace period for user: {}", user_id);
            //Rignt now we are doing nothing about it
        }
        subscription_notification_type::SUBSCRIPTION_RESTARTED => {
            println!("Subscription restarted for user: {}", user_id);
            // it is automatically handled in renewal flow
        }
        subscription_notification_type::SUBSCRIPTION_PRICE_CHANGE_CONFIRMED => {
            println!("Subscription price change confirmed for user: {}", user_id);
            // right now we are not doing anything about it
        }
        subscription_notification_type::SUBSCRIPTION_DEFERRED => {
            println!("Subscription deferred for user: {}", user_id);
            // not doing anything about it right now
        }
        subscription_notification_type::SUBSCRIPTION_PAUSED => {
            // we are not supporting subscription pause right now
            println!("Subscription paused for user: {}", user_id);
        }
        subscription_notification_type::SUBSCRIPTION_PAUSE_SCHEDULE_CHANGED => {
            println!("Subscription pause schedule changed for user: {}", user_id);
            // we are not supporting subscription pause right now
        }
        subscription_notification_type::SUBSCRIPTION_REVOKED
        | subscription_notification_type::SUBSCRIPTION_EXPIRED
        | subscription_notification_type::SUBSCRIPTION_ON_HOLD => {
            handle_revoking_user_access(
                &mut app_state
                    .get_db_connection()
                    .map_err(|_| AppError::DatabaseConnection)?,
                app_state
                    .admin_ic_agent
                    .as_ref()
                    .ok_or(AppError::AdminIcAgentMissing)?,
                &user_id,
                purchase_token,
                &google_play_subscription_response,
            )
            .await?;
            println!("Subscription revoked for user: {}", user_id);
        }
        _ => {
            println!(
                "Unknown subscription notification type: {} for user: {}",
                notification_type, user_id
            );
        }
    }

    Ok(())
}

fn handle_linked_purchase_token(
    database_conn: &mut PooledConnection<ConnectionManager<SqliteConnection>>,
    linked_purchase_token: Option<String>,
) -> Result<(), AppError> {
    use crate::schema::purchase_tokens::dsl::*;

    if let Some(token) = linked_purchase_token {
        diesel::update(purchase_tokens.filter(purchase_token.eq(&token)))
            .set(status.eq(PurchaseTokenStatus::Expired))
            .execute(database_conn)
            .map_err(|_| AppError::DatabaseConnection)?;
    }

    Ok(())
}

async fn handle_test_notification(
    notification: &crate::types::TestNotification,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!(
        "Test notification received - Version: {}",
        notification.version
    );
    println!("This is a test notification from Google Play Console");

    Ok(())
}
