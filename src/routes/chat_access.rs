use crate::error::{AppError, AppResult};
use crate::model::BotChatAccess;
use crate::routes::goole_play_billing_helpers::{
    consume_google_play_product, fetch_google_play_product_details,
};
use crate::types::{
    google_play_product_purchase_state, ApiResponse, BotChatAccessStatus, ChatAccessResponse,
    EmptyData, GrantChatAccessRequest,
};
use crate::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use diesel::prelude::*;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct CheckChatAccessQuery {
    pub user_id: String,
    pub bot_id: String,
}

#[utoipa::path(
    post,
    path = "/google/chat-access/grant",
    request_body = GrantChatAccessRequest,
    responses(
        (status = 200, description = "Chat access granted successfully", body = ApiResponse<EmptyData>),
        (status = 400, description = "Invalid or already-used purchase token", body = ApiResponse<EmptyData>),
        (status = 500, description = "Internal server error", body = ApiResponse<EmptyData>)
    ),
    tag = "Chat Access"
)]
pub async fn grant_chat_access(
    State(app_state): State<AppState>,
    Json(payload): Json<GrantChatAccessRequest>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = app_state.get_db_connection()?;

    process_grant_chat_access(&mut conn, &app_state, &payload).await?;

    Ok((
        StatusCode::OK,
        Json(ApiResponse::<EmptyData>::success(EmptyData {})),
    ))
}

async fn process_grant_chat_access(
    conn: &mut SqliteConnection,
    app_state: &AppState,
    payload: &GrantChatAccessRequest,
) -> AppResult<()> {
    use crate::schema::bot_chat_access::dsl::*;

    // Idempotency: if this purchase token already has a grant, return early or error
    let existing: Option<BotChatAccess> = bot_chat_access
        .filter(purchase_token.eq(&payload.purchase_token))
        .first(conn)
        .optional()?;

    if let Some(grant) = existing {
        if grant.bot_id != payload.bot_id {
            return Err(AppError::TokenAlreadyUsed);
        }
        // Same bot, same token — already granted, idempotent success
        return Ok(());
    }

    let product_response = fetch_google_play_product_details(
        &payload.package_name,
        &payload.purchase_token,
        app_state.google_auth.as_ref(),
    )
    .await?;

    if product_response.purchase_state
        != google_play_product_purchase_state::PURCHASE_STATE_PURCHASED
    {
        return Err(AppError::BadRequest(
            "Purchase is not in purchased state".to_string(),
        ));
    }

    let user_id_str = product_response
        .obfuscated_external_account_id
        .ok_or(AppError::ExternalAccountIdentifiersMissing)?;

    let access_expires_at = chrono::Utc::now().naive_utc() + chrono::Duration::hours(24);

    let new_grant = BotChatAccess::new(
        payload.purchase_token.clone(),
        user_id_str,
        payload.bot_id.clone(),
        access_expires_at,
    );

    diesel::insert_into(bot_chat_access)
        .values(&new_grant)
        .execute(conn)?;

    consume_google_play_product(
        &payload.package_name,
        &payload.product_id,
        &payload.purchase_token,
        app_state.google_auth.as_ref(),
    )
    .await?;

    Ok(())
}

#[utoipa::path(
    get,
    path = "/google/chat-access/check",
    params(
        ("user_id" = String, Query, description = "User ID to check access for"),
        ("bot_id" = String, Query, description = "Bot ID to check access for"),
    ),
    responses(
        (status = 200, description = "Access check result", body = ApiResponse<ChatAccessResponse>),
        (status = 500, description = "Internal server error", body = ApiResponse<EmptyData>)
    ),
    tag = "Chat Access"
)]
pub async fn check_chat_access(
    State(app_state): State<AppState>,
    Query(params): Query<CheckChatAccessQuery>,
) -> Result<impl IntoResponse, AppError> {
    use crate::schema::bot_chat_access::dsl::*;

    let mut conn = app_state.get_db_connection()?;

    let now = chrono::Utc::now().naive_utc();

    let grant: Option<BotChatAccess> = bot_chat_access
        .filter(user_id.eq(&params.user_id))
        .filter(bot_id.eq(&params.bot_id))
        .filter(status.eq(BotChatAccessStatus::Active))
        .filter(expires_at.gt(now))
        .order(expires_at.desc())
        .first(&mut conn)
        .optional()?;

    let response = match grant {
        Some(g) => ChatAccessResponse {
            has_access: true,
            expires_at: Some(
                chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                    g.expires_at,
                    chrono::Utc,
                )
                .to_rfc3339(),
            ),
        },
        None => ChatAccessResponse {
            has_access: false,
            expires_at: None,
        },
    };

    Ok((StatusCode::OK, Json(ApiResponse::success(response))))
}
