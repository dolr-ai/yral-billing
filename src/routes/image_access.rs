use crate::consts::{IMAGE_UNLOCK_PRODUCT_ID, IMAGE_UNLOCK_REWARD_PAISE};
use crate::error::{AppError, AppResult};
use crate::model::{ImageAccess, Transaction};
use crate::routes::goole_play_billing_helpers::{
    consume_google_play_product, fetch_google_play_product_details,
};
use crate::types::{
    google_play_consumption_state, google_play_product_purchase_state, ApiResponse,
    BotChatAccessStatus, EmptyData, GrantImageAccessRequest, ImageAccessCheckBatchRequest,
    ImageAccessCheckBatchResponse, PurchaseSource, TransactionType,
};
use crate::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use diesel::prelude::*;
use std::collections::HashMap;

pub const MAX_IMAGE_ID_LEN: usize = 512;
pub const MAX_CHECK_BATCH_IDS: usize = 200;

pub(crate) fn validate_image_unlock_request(product_id: &str, image_id: &str) -> AppResult<()> {
    if product_id != IMAGE_UNLOCK_PRODUCT_ID {
        return Err(AppError::BadRequest(format!(
            "Invalid product id for image unlock: {product_id}"
        )));
    }
    if image_id.is_empty() || image_id.len() > MAX_IMAGE_ID_LEN {
        return Err(AppError::BadRequest(format!(
            "image_id must be 1..={MAX_IMAGE_ID_LEN} characters"
        )));
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/google/image-access/grant",
    request_body = GrantImageAccessRequest,
    responses(
        (status = 200, description = "Image access granted successfully", body = ApiResponse<EmptyData>),
        (status = 400, description = "Invalid or already-used purchase token", body = ApiResponse<EmptyData>),
        (status = 500, description = "Internal server error", body = ApiResponse<EmptyData>)
    ),
    tag = "Image Access"
)]
pub async fn grant_image_access(
    State(app_state): State<AppState>,
    Json(payload): Json<GrantImageAccessRequest>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = app_state.get_db_connection()?;

    process_grant_image_access(&mut conn, &app_state, &payload).await?;

    Ok((
        StatusCode::OK,
        Json(ApiResponse::<EmptyData>::success(EmptyData {})),
    ))
}

async fn process_grant_image_access(
    conn: &mut SqliteConnection,
    app_state: &AppState,
    payload: &GrantImageAccessRequest,
) -> AppResult<()> {
    use crate::schema::image_access::dsl::*;

    validate_image_unlock_request(&payload.product_id, &payload.image_id)?;

    let existing: Option<ImageAccess> = image_access
        .filter(purchase_source.eq(PurchaseSource::Google))
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

            let new_grant = ImageAccess::new(
                PurchaseSource::Google,
                payload.purchase_token.clone(),
                user_id_str,
                payload.bot_id.clone(),
                payload.image_id.clone(),
            );

            diesel::insert_into(image_access)
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
            diesel::update(image_access.filter(id.eq(&new_grant.id)))
                .set((status.eq(BotChatAccessStatus::Active), updated_at.eq(now)))
                .execute(conn)?;

            let reward = Transaction::new(
                new_grant.user_id.clone(),
                TransactionType::ImageUnlockReward,
                IMAGE_UNLOCK_REWARD_PAISE,
                payload.bot_id.clone(),
                PurchaseSource::Google,
                payload.purchase_token.clone(),
            );
            diesel::insert_into(crate::schema::transactions::table)
                .values(&reward)
                .execute(conn)?;

            Ok(())
        }

        // ── Token reused for a different image: always reject ──
        Some(grant) if grant.image_id != payload.image_id => Err(AppError::TokenAlreadyUsed),

        // ── Same token, same image: apply state machine ──
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

                    Some(state_str) => {
                        return Err(AppError::BadRequest(format!(
                            "Unexpected consumption state: {state_str}"
                        )));
                    }
                }

                let now = chrono::Utc::now().naive_utc();
                diesel::update(image_access.filter(id.eq(&grant.id)))
                    .set((status.eq(BotChatAccessStatus::Active), updated_at.eq(now)))
                    .execute(conn)?;

                let reward_exists: bool = diesel::select(diesel::dsl::exists(
                    crate::schema::transactions::table
                        .filter(
                            crate::schema::transactions::purchase_source.eq(PurchaseSource::Google),
                        )
                        .filter(
                            crate::schema::transactions::purchase_token.eq(&payload.purchase_token),
                        ),
                ))
                .get_result(conn)?;

                if !reward_exists {
                    let reward = Transaction::new(
                        grant.user_id.clone(),
                        TransactionType::ImageUnlockReward,
                        IMAGE_UNLOCK_REWARD_PAISE,
                        payload.bot_id.clone(),
                        PurchaseSource::Google,
                        payload.purchase_token.clone(),
                    );
                    diesel::insert_into(crate::schema::transactions::table)
                        .values(&reward)
                        .execute(conn)?;
                }

                Ok(())
            }

            // Access is permanent — idempotent success
            BotChatAccessStatus::Active => Ok(()),

            // Token was canceled (e.g. refund) — terminal state
            BotChatAccessStatus::Canceled => Err(AppError::TokenAlreadyUsed),

            // Never written for image rows; defensive terminal state
            BotChatAccessStatus::Expired => Err(AppError::TokenExpired),
        },
    }
}

#[utoipa::path(
    post,
    path = "/image-access/check-batch",
    request_body = ImageAccessCheckBatchRequest,
    responses(
        (status = 200, description = "Per-image access map", body = ApiResponse<ImageAccessCheckBatchResponse>),
        (status = 400, description = "Empty or oversized image id list", body = ApiResponse<EmptyData>),
        (status = 500, description = "Internal server error", body = ApiResponse<EmptyData>)
    ),
    tag = "Image Access"
)]
pub async fn check_image_access_batch(
    State(app_state): State<AppState>,
    Json(payload): Json<ImageAccessCheckBatchRequest>,
) -> Result<impl IntoResponse, AppError> {
    use crate::schema::image_access::dsl::*;

    if payload.image_ids.is_empty() {
        return Err(AppError::BadRequest("image_ids must not be empty".into()));
    }

    let mut requested: Vec<String> = payload.image_ids.clone();
    requested.sort();
    requested.dedup();

    if requested.len() > MAX_CHECK_BATCH_IDS {
        return Err(AppError::BadRequest(format!(
            "image_ids must contain at most {MAX_CHECK_BATCH_IDS} unique ids"
        )));
    }

    let mut conn = app_state.get_db_connection()?;

    // An active bot subscription makes every image in that bot's chat free.
    if let Some(subscribed_bot_id) = payload.bot_id.as_deref() {
        if crate::routes::bot_subscription::find_active_subscription(
            &mut conn,
            &payload.user_id,
            subscribed_bot_id,
        )?
        .is_some()
        {
            let access: HashMap<String, bool> =
                requested.into_iter().map(|img| (img, true)).collect();
            return Ok((
                StatusCode::OK,
                Json(ApiResponse::success(ImageAccessCheckBatchResponse {
                    access,
                })),
            ));
        }
    }

    // Source-agnostic: a purchase from either store unlocks the image everywhere.
    let unlocked: Vec<String> = image_access
        .filter(user_id.eq(&payload.user_id))
        .filter(status.eq(BotChatAccessStatus::Active))
        .filter(image_id.eq_any(&requested))
        .select(image_id)
        .load(&mut conn)?;

    let unlocked: std::collections::HashSet<String> = unlocked.into_iter().collect();

    let access: HashMap<String, bool> = requested
        .into_iter()
        .map(|img| {
            let has = unlocked.contains(&img);
            (img, has)
        })
        .collect();

    Ok((
        StatusCode::OK,
        Json(ApiResponse::success(ImageAccessCheckBatchResponse {
            access,
        })),
    ))
}
