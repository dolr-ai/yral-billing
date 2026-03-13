use crate::error::{AppError, AppResult};
use crate::model::BotChatAccess;
use crate::routes::goole_play_billing_helpers::{
    consume_google_play_product, fetch_google_play_product_details,
};
use crate::types::{
    google_play_consumption_state, google_play_product_purchase_state, ApiResponse,
    BotChatAccessStatus, ChatAccessResponse, EmptyData, GrantChatAccessRequest,
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

    let existing: Option<BotChatAccess> = bot_chat_access
        .filter(purchase_token.eq(&payload.purchase_token))
        .first(conn)
        .optional()?;

    match existing {
        // ── No row yet: validate purchase, insert as ConsumePending, then consume ──
        None => {
            let product_response = fetch_google_play_product_details(
                &payload.package_name,
                &payload.purchase_token,
                app_state.google_auth.as_ref(),
            )
            .await?;

            let line_item = product_response
                .product_line_item
                .as_deref()
                .and_then(|items| items.first());

            if line_item.map(|i| i.product_id.as_str()) != Some(payload.product_id.as_str()) {
                return Err(AppError::BadRequest(format!(
                    "Product id mismatch: expected {}, got {:?}",
                    payload.product_id,
                    line_item.map(|i| &i.product_id)
                )));
            }

            if product_response
                .purchase_state_context
                .as_ref()
                .and_then(|c| c.purchase_state.as_deref())
                != Some(google_play_product_purchase_state::PURCHASE_STATE_PURCHASED)
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

            let now = chrono::Utc::now().naive_utc();
            diesel::update(bot_chat_access.filter(id.eq(&new_grant.id)))
                .set((status.eq(BotChatAccessStatus::Active), updated_at.eq(now)))
                .execute(conn)?;

            Ok(())
        }

        // ── Token reused for a different bot: always reject ──
        Some(grant) if grant.bot_id != payload.bot_id => Err(AppError::TokenAlreadyUsed),

        // ── Same token, same bot: apply state machine ──
        Some(grant) => match grant.status {
            // Consume was attempted before but not confirmed — resume from where we left off
            BotChatAccessStatus::ConsumePending => {
                let product_response = fetch_google_play_product_details(
                    &payload.package_name,
                    &payload.purchase_token,
                    app_state.google_auth.as_ref(),
                )
                .await?;

                let line_item = product_response
                    .product_line_item
                    .as_deref()
                    .and_then(|items| items.first());

                if line_item.map(|i| i.product_id.as_str()) != Some(payload.product_id.as_str()) {
                    return Err(AppError::BadRequest(format!(
                        "Product id mismatch: expected {}, got {:?}",
                        payload.product_id,
                        line_item.map(|i| &i.product_id)
                    )));
                }

                let consumption_state = line_item
                    .and_then(|i| i.product_offer_details.as_ref())
                    .and_then(|o| o.consumption_state.as_deref());

                match consumption_state {
                    // Google Play already consumed it on a prior attempt — just activate
                    Some(google_play_consumption_state::CONSUMED) => {}

                    // Not yet consumed — retry
                    Some(google_play_consumption_state::NOT_CONSUMED) | None => {
                        consume_google_play_product(
                            &payload.package_name,
                            &payload.product_id,
                            &payload.purchase_token,
                            app_state.google_auth.as_ref(),
                        )
                        .await?;
                    }

                    Some(state) => {
                        return Err(AppError::BadRequest(format!(
                            "Unexpected consumption state: {state}"
                        )));
                    }
                }

                let now = chrono::Utc::now().naive_utc();
                diesel::update(bot_chat_access.filter(id.eq(&grant.id)))
                    .set((status.eq(BotChatAccessStatus::Active), updated_at.eq(now)))
                    .execute(conn)?;

                Ok(())
            }

            // Access is live and within the window — idempotent success
            BotChatAccessStatus::Active if grant.expires_at > chrono::Utc::now().naive_utc() => {
                Ok(())
            }

            // Access window has passed — token is spent, new purchase required
            BotChatAccessStatus::Active => Err(AppError::TokenExpired),

            // Token was canceled (e.g. refund) — terminal state
            BotChatAccessStatus::Canceled => Err(AppError::TokenAlreadyUsed),

            // Expired status set explicitly (e.g. by a background job) — terminal state
            BotChatAccessStatus::Expired => Err(AppError::TokenExpired),
        },
    }
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
