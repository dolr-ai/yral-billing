use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use diesel::prelude::*;

use crate::{
    consts::{BOT_SUBSCRIPTION_REWARD_PAISE, CHAT_ACCESS_PRODUCT_IDS},
    error::{AppError, AppResult},
    model::{AppleAppAccountToken, BotChatAccess, Transaction},
    routes::apple_billing_helpers::{
        decode_apple_notification_jws, decode_apple_transaction_jws, default_apple_environment,
        fetch_apple_transaction_info,
    },
    types::{
        ApiResponse, AppleAppAccountTokenRequest, AppleAppAccountTokenResponse,
        AppleServerNotificationRequest, BotChatAccessStatus, EmptyData,
        GrantAppleChatAccessRequest, PurchaseSource, TransactionType,
    },
    AppState,
};

#[utoipa::path(
    post,
    path = "/apple/app-account-token",
    request_body = AppleAppAccountTokenRequest,
    responses(
        (status = 200, description = "Stable Apple appAccountToken UUID", body = ApiResponse<AppleAppAccountTokenResponse>),
        (status = 500, description = "Internal server error", body = ApiResponse<EmptyData>)
    ),
    tag = "Chat Access"
)]
pub async fn get_or_create_apple_app_account_token(
    State(app_state): State<AppState>,
    Json(payload): Json<AppleAppAccountTokenRequest>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = app_state.get_db_connection()?;
    let token = get_or_create_app_account_token(&mut conn, &payload.user_id)?;

    Ok((
        StatusCode::OK,
        Json(ApiResponse::success(AppleAppAccountTokenResponse {
            app_account_token: token,
        })),
    ))
}

fn get_or_create_app_account_token(
    conn: &mut SqliteConnection,
    user_id_param: &str,
) -> AppResult<String> {
    use crate::schema::apple_app_account_tokens::dsl::*;

    let existing: Option<AppleAppAccountToken> = apple_app_account_tokens
        .filter(user_id.eq(user_id_param))
        .first(conn)
        .optional()?;

    if let Some(existing) = existing {
        return Ok(existing.app_account_token);
    }

    let new_mapping = AppleAppAccountToken::new(user_id_param.to_string());
    diesel::insert_into(apple_app_account_tokens)
        .values(&new_mapping)
        .execute(conn)?;

    Ok(new_mapping.app_account_token)
}

pub(crate) fn resolve_app_account_token(
    conn: &mut SqliteConnection,
    app_account_token_param: &str,
) -> AppResult<String> {
    use crate::schema::apple_app_account_tokens::dsl::*;

    let mapping: AppleAppAccountToken = apple_app_account_tokens
        .filter(app_account_token.eq(app_account_token_param))
        .first(conn)
        .optional()?
        .ok_or_else(|| AppError::AppleVerification("Unknown Apple appAccountToken".to_string()))?;

    Ok(mapping.user_id)
}

#[utoipa::path(
    post,
    path = "/apple/chat-access/grant",
    request_body = GrantAppleChatAccessRequest,
    responses(
        (status = 200, description = "iOS chat access granted successfully", body = ApiResponse<EmptyData>),
        (status = 400, description = "Invalid or already-used Apple transaction", body = ApiResponse<EmptyData>),
        (status = 500, description = "Internal server error", body = ApiResponse<EmptyData>)
    ),
    tag = "Chat Access"
)]
pub async fn grant_apple_chat_access(
    State(app_state): State<AppState>,
    Json(payload): Json<GrantAppleChatAccessRequest>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = app_state.get_db_connection()?;
    process_grant_apple_chat_access(&mut conn, &payload).await?;

    Ok((
        StatusCode::OK,
        Json(ApiResponse::<EmptyData>::success(EmptyData {})),
    ))
}

async fn process_grant_apple_chat_access(
    conn: &mut SqliteConnection,
    payload: &GrantAppleChatAccessRequest,
) -> AppResult<()> {
    use crate::schema::bot_chat_access::dsl::*;

    if !CHAT_ACCESS_PRODUCT_IDS.contains(&payload.product_id.as_str()) {
        return Err(AppError::BadRequest(format!(
            "Invalid product id for chat access: {}",
            payload.product_id
        )));
    }

    let stored_purchase_token = payload.transaction_id.clone();
    let existing: Option<BotChatAccess> = bot_chat_access
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
            let access_expires_at = chrono::Utc::now().naive_utc() + chrono::Duration::hours(24);
            let new_grant = BotChatAccess::new(
                PurchaseSource::Apple,
                stored_purchase_token.clone(),
                user_id_str,
                payload.bot_id.clone(),
                access_expires_at,
            );

            diesel::insert_into(bot_chat_access)
                .values(&new_grant)
                .execute(conn)?;

            let now = chrono::Utc::now().naive_utc();
            diesel::update(bot_chat_access.filter(id.eq(&new_grant.id)))
                .set((status.eq(BotChatAccessStatus::Active), updated_at.eq(now)))
                .execute(conn)?;

            let reward = Transaction::new(
                new_grant.user_id.clone(),
                TransactionType::BotSubscriptionReward,
                BOT_SUBSCRIPTION_REWARD_PAISE,
                payload.bot_id.clone(),
                PurchaseSource::Apple,
                stored_purchase_token,
            );
            diesel::insert_into(crate::schema::transactions::table)
                .values(&reward)
                .execute(conn)?;

            Ok(())
        }
        Some(grant) if grant.bot_id != payload.bot_id => Err(AppError::TokenAlreadyUsed),
        Some(grant) => match grant.status {
            BotChatAccessStatus::ConsumePending => {
                let now = chrono::Utc::now().naive_utc();
                diesel::update(bot_chat_access.filter(id.eq(&grant.id)))
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
                        TransactionType::BotSubscriptionReward,
                        BOT_SUBSCRIPTION_REWARD_PAISE,
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
            BotChatAccessStatus::Active if grant.expires_at > chrono::Utc::now().naive_utc() => {
                Ok(())
            }
            BotChatAccessStatus::Active => Err(AppError::TokenExpired),
            BotChatAccessStatus::Canceled => Err(AppError::TokenAlreadyUsed),
            BotChatAccessStatus::Expired => Err(AppError::TokenExpired),
        },
    }
}

#[utoipa::path(
    post,
    path = "/apple/server-notifications",
    request_body = AppleServerNotificationRequest,
    responses(
        (status = 200, description = "Apple notification accepted"),
        (status = 400, description = "Invalid Apple notification")
    ),
    tag = "Chat Access"
)]
pub async fn handle_apple_server_notification(
    State(app_state): State<AppState>,
    Json(payload): Json<AppleServerNotificationRequest>,
) -> Result<impl IntoResponse, AppError> {
    process_apple_server_notification(&app_state, &payload.signed_payload).await?;
    Ok((StatusCode::OK, "OK"))
}

async fn process_apple_server_notification(
    app_state: &AppState,
    signed_payload: &str,
) -> AppResult<()> {
    let notification = decode_apple_notification_jws(signed_payload)?;

    if notification.notification_type == "TEST" {
        return Ok(());
    }

    let should_cancel = matches!(notification.notification_type.as_str(), "REFUND" | "REVOKE");

    if !should_cancel {
        return Ok(());
    }

    let Some(data) = notification.data else {
        return Ok(());
    };
    let Some(signed_transaction_info) = data.signed_transaction_info else {
        return Ok(());
    };

    let transaction = decode_apple_transaction_jws(&signed_transaction_info)?;
    let stored_purchase_token = transaction.transaction_id;

    use crate::schema::bot_chat_access::dsl;
    use crate::schema::image_access::dsl as image_dsl;
    let mut conn = app_state.get_db_connection()?;
    let now = chrono::Utc::now().naive_utc();

    // A transaction id maps to exactly one purchase, so at most one of these
    // updates matches a row; running both unconditionally is safe.
    diesel::update(
        dsl::bot_chat_access
            .filter(dsl::purchase_source.eq(PurchaseSource::Apple))
            .filter(dsl::purchase_token.eq(&stored_purchase_token)),
    )
    .set((
        dsl::status.eq(BotChatAccessStatus::Canceled),
        dsl::updated_at.eq(now),
    ))
    .execute(&mut conn)?;

    diesel::update(
        image_dsl::image_access
            .filter(image_dsl::purchase_source.eq(PurchaseSource::Apple))
            .filter(image_dsl::purchase_token.eq(&stored_purchase_token)),
    )
    .set((
        image_dsl::status.eq(BotChatAccessStatus::Canceled),
        image_dsl::updated_at.eq(now),
    ))
    .execute(&mut conn)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::get_or_create_app_account_token;
    use diesel::prelude::*;
    use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

    const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

    #[test]
    fn app_account_tokens_are_stable_for_user() {
        let db_path = format!("./test_apple_token_{}.db", uuid::Uuid::new_v4());
        let mut conn = SqliteConnection::establish(&db_path).unwrap();
        conn.run_pending_migrations(MIGRATIONS).unwrap();

        let first = get_or_create_app_account_token(&mut conn, "user-1").unwrap();
        let second = get_or_create_app_account_token(&mut conn, "user-1").unwrap();

        assert_eq!(first, second);
        assert!(uuid::Uuid::parse_str(&first).is_ok());

        let _ = std::fs::remove_file(db_path);
    }
}
