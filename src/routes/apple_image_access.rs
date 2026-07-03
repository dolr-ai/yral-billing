use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use diesel::prelude::*;

use crate::{
    consts::IMAGE_UNLOCK_REWARD_PAISE,
    error::{AppError, AppResult},
    model::{ImageAccess, Transaction},
    routes::apple_billing_helpers::{default_apple_environment, fetch_apple_transaction_info},
    routes::apple_chat_access::resolve_app_account_token,
    routes::image_access::validate_image_unlock_request,
    types::{
        ApiResponse, BotChatAccessStatus, EmptyData, GrantAppleImageAccessRequest, PurchaseSource,
        TransactionType,
    },
    AppState,
};

#[utoipa::path(
    post,
    path = "/apple/image-access/grant",
    request_body = GrantAppleImageAccessRequest,
    responses(
        (status = 200, description = "iOS image access granted successfully", body = ApiResponse<EmptyData>),
        (status = 400, description = "Invalid or already-used Apple transaction", body = ApiResponse<EmptyData>),
        (status = 500, description = "Internal server error", body = ApiResponse<EmptyData>)
    ),
    tag = "Image Access"
)]
pub async fn grant_apple_image_access(
    State(app_state): State<AppState>,
    Json(payload): Json<GrantAppleImageAccessRequest>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = app_state.get_db_connection()?;
    process_grant_apple_image_access(&mut conn, &payload).await?;

    Ok((
        StatusCode::OK,
        Json(ApiResponse::<EmptyData>::success(EmptyData {})),
    ))
}

async fn process_grant_apple_image_access(
    conn: &mut SqliteConnection,
    payload: &GrantAppleImageAccessRequest,
) -> AppResult<()> {
    use crate::schema::image_access::dsl::*;

    validate_image_unlock_request(&payload.product_id, &payload.image_id)?;

    let stored_purchase_token = payload.transaction_id.clone();
    let existing: Option<ImageAccess> = image_access
        .filter(purchase_source.eq(PurchaseSource::Apple))
        .filter(purchase_token.eq(&stored_purchase_token))
        .first(conn)
        .optional()?;

    match existing {
        None => {
            let transaction = fetch_apple_transaction_info(
                &payload.transaction_id,
                &payload.product_id,
                payload
                    .environment
                    .unwrap_or_else(default_apple_environment),
            )
            .await?;

            let app_account_token = transaction
                .app_account_token
                .ok_or(AppError::ExternalAccountIdentifiersMissing)?;
            let user_id_str = resolve_app_account_token(conn, &app_account_token)?;
            let new_grant = ImageAccess::new(
                PurchaseSource::Apple,
                stored_purchase_token.clone(),
                user_id_str,
                payload.bot_id.clone(),
                payload.image_id.clone(),
            );

            diesel::insert_into(image_access)
                .values(&new_grant)
                .execute(conn)?;

            let now = chrono::Utc::now().naive_utc();
            diesel::update(image_access.filter(id.eq(&new_grant.id)))
                .set((status.eq(BotChatAccessStatus::Active), updated_at.eq(now)))
                .execute(conn)?;

            let reward = Transaction::new(
                new_grant.user_id.clone(),
                TransactionType::ImageUnlockReward,
                IMAGE_UNLOCK_REWARD_PAISE,
                payload.bot_id.clone(),
                PurchaseSource::Apple,
                stored_purchase_token,
            );
            diesel::insert_into(crate::schema::transactions::table)
                .values(&reward)
                .execute(conn)?;

            Ok(())
        }
        Some(grant) if grant.image_id != payload.image_id => Err(AppError::TokenAlreadyUsed),
        Some(grant) => match grant.status {
            BotChatAccessStatus::ConsumePending => {
                let now = chrono::Utc::now().naive_utc();
                diesel::update(image_access.filter(id.eq(&grant.id)))
                    .set((status.eq(BotChatAccessStatus::Active), updated_at.eq(now)))
                    .execute(conn)?;

                let reward_exists: bool = diesel::select(diesel::dsl::exists(
                    crate::schema::transactions::table
                        .filter(
                            crate::schema::transactions::purchase_source.eq(PurchaseSource::Apple),
                        )
                        .filter(
                            crate::schema::transactions::purchase_token.eq(&stored_purchase_token),
                        ),
                ))
                .get_result(conn)?;

                if !reward_exists {
                    let reward = Transaction::new(
                        grant.user_id.clone(),
                        TransactionType::ImageUnlockReward,
                        IMAGE_UNLOCK_REWARD_PAISE,
                        payload.bot_id.clone(),
                        PurchaseSource::Apple,
                        stored_purchase_token,
                    );
                    diesel::insert_into(crate::schema::transactions::table)
                        .values(&reward)
                        .execute(conn)?;
                }

                Ok(())
            }
            // Access is permanent — idempotent success
            BotChatAccessStatus::Active => Ok(()),
            BotChatAccessStatus::Canceled => Err(AppError::TokenAlreadyUsed),
            // Never written for image rows; defensive terminal state
            BotChatAccessStatus::Expired => Err(AppError::TokenExpired),
        },
    }
}
