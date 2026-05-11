use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use diesel::prelude::*;

use crate::{
    consts::BOT_SUBSCRIPTION_REWARD_PAISE,
    error::{AppError, AppResult},
    model::{BotChatAccess, Transaction},
    routes::apple_billing_helpers::{
        decode_apple_notification_jws, decode_apple_transaction_jws, default_apple_environment,
        fetch_apple_transaction_info,
    },
    types::{
        ApiResponse, AppleServerNotificationRequest, BotChatAccessStatus, EmptyData,
        GrantAppleChatAccessRequest, TransactionType,
    },
    AppState,
};

fn apple_purchase_token(transaction_id: &str) -> String {
    format!("apple:{transaction_id}")
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

    let stored_purchase_token = apple_purchase_token(&payload.transaction_id);
    let existing: Option<BotChatAccess> = bot_chat_access
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

            let user_id_str = transaction
                .app_account_token
                .ok_or(AppError::ExternalAccountIdentifiersMissing)?;
            let access_expires_at = chrono::Utc::now().naive_utc() + chrono::Duration::hours(24);
            let new_grant = BotChatAccess::new(
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
                    crate::schema::transactions::table.filter(
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
    let stored_purchase_token = apple_purchase_token(&transaction.transaction_id);

    use crate::schema::bot_chat_access::dsl;
    let mut conn = app_state.get_db_connection()?;
    let now = chrono::Utc::now().naive_utc();

    diesel::update(dsl::bot_chat_access.filter(dsl::purchase_token.eq(stored_purchase_token)))
        .set((
            dsl::status.eq(BotChatAccessStatus::Canceled),
            dsl::updated_at.eq(now),
        ))
        .execute(&mut conn)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::apple_purchase_token;

    #[test]
    fn apple_purchase_tokens_are_namespaced() {
        assert_eq!(apple_purchase_token("12345"), "apple:12345");
    }
}
