use crate::auth::GoogleAuth;
use crate::error::{AppError, AppResult};
use crate::model::PurchaseToken;
use crate::routes::goole_play_billing_helpers::{
    acknowledge_google_play, fetch_google_play_purchase_details,
};
use crate::routes::purchase_token_helpers::verify_subcription_response_for_active_status;
use crate::types::{
    ApiResponse, EmptyData, GooglePlaySubscriptionResponse, PurchaseTokenStatus, VerifyRequest,
};

#[cfg(any(feature = "local", feature = "mock-google-api"))]
use crate::types::SubscriptionLineItem;
use crate::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use diesel::prelude::*;
use std::sync::Arc;
use utoipa;

fn verify_purchase_token_validity_for_subscription_active(
    payload: &VerifyRequest,
    subscription_response: &GooglePlaySubscriptionResponse,
) -> AppResult<()> {
    subscription_response
        .line_items
        .iter()
        .find(|item| item.product_id == payload.product_id)
        .ok_or(AppError::SubscriptionInvalidLineItems)?;

    verify_subcription_response_for_active_status(subscription_response)
}

/// Grant user access to your services after successful purchase acknowledgment
async fn grant_user_access(
    admin_ic_agent: Option<&ic_agent::Agent>,
    user_id: &str,
) -> AppResult<()> {
    #[cfg(feature = "local")]
    {
        // Mock service call for development/testing
        println!("MOCK: Granting access to user {}", user_id);
        Ok(())
    }

    #[cfg(not(feature = "local"))]
    {
        use crate::routes::utils::grant_yral_pro_plan_access;

        let Some(admin_ic_agent) = admin_ic_agent else {
            return Err(AppError::InternalError(
                "Admin IC agent not available".to_string(),
            ));
        };

        grant_yral_pro_plan_access(admin_ic_agent, user_id).await?;

        Ok(())
    }
}

async fn process_purchase_token(
    conn: &mut SqliteConnection,
    auth: Option<&Arc<GoogleAuth>>,
    admin_ic_agent: Option<&ic_agent::Agent>,
    payload: &VerifyRequest,
) -> AppResult<()> {
    use crate::schema::purchase_tokens::dsl::*;

    let existing_token: Option<PurchaseToken> = purchase_tokens
        .filter(purchase_token.eq(&payload.purchase_token))
        .first(conn)
        .optional()?;

    match existing_token {
        Some(token) if token.user_id != payload.user_id => {
            return Err(AppError::TokenAlreadyUsed);
        }
        Some(token)
            if token.status == PurchaseTokenStatus::AccessGranted
                && token.expiry_at > chrono::Utc::now().naive_utc() =>
        {
            Ok(())
        }
        _ => {
            let gooogle_subscription_response = fetch_google_play_purchase_details(
                &payload.package_name,
                &payload.purchase_token,
                auth,
            )
            .await?;

            verify_purchase_token_validity_for_subscription_active(
                payload,
                &gooogle_subscription_response,
            )?;

            acknowledge_google_play(
                &payload.package_name,
                &payload.purchase_token,
                &gooogle_subscription_response,
                auth,
            )
            .await?;

            grant_user_access(
                admin_ic_agent,
                gooogle_subscription_response
                    .external_account_identifiers
                    .ok_or(AppError::ExternalAccountIdentifiersMissing)?
                    .obfuscated_external_account_id
                    .ok_or(AppError::ExternalAccountIdentifiersMissing)?
                    .as_str(),
            )
            .await?;

            let expiry = gooogle_subscription_response
                .line_items
                .iter()
                .find(|item| item.product_id == payload.product_id)
                .map(|item| item.expiry_time.clone())
                .ok_or(AppError::SubscriptionInvalidLineItems)?;

            let expiry_native = expiry
                .and_then(|time_str| chrono::DateTime::parse_from_rfc3339(&time_str).ok())
                .map(|dt| dt.naive_utc())
                .ok_or(AppError::SubscriptionInvalidLineItems)?;

            let new_token = PurchaseToken::new(
                payload.user_id.clone(),
                payload.purchase_token.clone(),
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

#[utoipa::path(
    post,
    path = "/google/verify",
    request_body = VerifyRequest,
    responses(
        (status = 200, description = "Subscription verification successful", body = ApiResponse<EmptyData>),
        (status = 400, description = "Bad request - subscription canceled, expired, or invalid", body = ApiResponse<EmptyData>),
        (status = 500, description = "Internal server error", body = ApiResponse<EmptyData>)
    ),
    tag = "Subscription Verification"
)]
pub async fn verify_purchase(
    State(app_state): State<AppState>,
    Json(payload): Json<VerifyRequest>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = app_state
        .get_db_connection()
        .map_err(|_| AppError::DatabaseConnection)?;

    process_purchase_token(
        &mut conn,
        app_state.google_auth.as_ref(),
        app_state.admin_ic_agent.as_ref(),
        &payload,
    )
    .await?;

    Ok((
        StatusCode::OK,
        Json(ApiResponse::<EmptyData>::success(EmptyData {})),
    ))
}
