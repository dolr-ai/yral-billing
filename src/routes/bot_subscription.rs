use crate::consts::{
    is_bot_subscription_product, BOT_SUBSCRIPTION_INITIAL_REWARD_PAISE,
    BOT_SUBSCRIPTION_RENEWAL_REWARD_PAISE,
};
use crate::error::{AppError, AppResult};
use crate::model::{BotSubscription, Transaction};
use crate::routes::apple_billing_helpers::{
    default_apple_environment, fetch_apple_transaction_info,
};
use crate::routes::apple_chat_access::resolve_app_account_token;
use crate::routes::goole_play_billing_helpers::{
    acknowledge_google_play, fetch_google_play_purchase_details,
};
use crate::routes::purchase_token_helpers::verify_subcription_response_for_active_status;
use crate::types::{
    subscription_notification_type, ApiResponse, AppleJWSTransactionDecodedPayload,
    AppleNotificationDecodedPayload, BotSubscriptionCheckResponse, BotSubscriptionStatus,
    EmptyData, GooglePlaySubscriptionResponse, GrantAppleBotSubscriptionRequest, PurchaseSource,
    TransactionType, VerifyBotSubscriptionRequest,
};
use crate::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct CheckBotSubscriptionQuery {
    pub user_id: String,
    pub bot_id: String,
}

/// Expiry of the line item matching `expected_product_id`, from the store's
/// RFC3339 `expiryTime`. The backend never computes subscription durations.
fn parse_google_expiry(
    response: &GooglePlaySubscriptionResponse,
    expected_product_id: &str,
) -> AppResult<NaiveDateTime> {
    response
        .line_items
        .iter()
        .find(|item| item.product_id == expected_product_id)
        .and_then(|item| item.expiry_time.as_deref())
        .and_then(|time_str| chrono::DateTime::parse_from_rfc3339(time_str).ok())
        .map(|dt| dt.naive_utc())
        .ok_or(AppError::SubscriptionInvalidLineItems)
}

fn apple_expiry(transaction: &AppleJWSTransactionDecodedPayload) -> AppResult<NaiveDateTime> {
    transaction
        .expires_date
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|dt| dt.naive_utc())
        .ok_or_else(|| {
            AppError::AppleVerification(
                "Transaction has no expiresDate; not an auto-renewable subscription".to_string(),
            )
        })
}

/// Inserts a creator reward unless one already exists for `dedup_token`.
///
/// Subscription rewards store an order-scoped id in `transactions.purchase_token`
/// (Google `latestOrderId`, Apple `transactionId` — both change per renewal),
/// so the same (source, token) exists-check used by the consumable flows
/// dedups initial and renewal rewards alike.
pub(crate) fn insert_subscription_reward_if_new(
    conn: &mut SqliteConnection,
    payer_user_id: &str,
    reward_bot_id: &str,
    source: PurchaseSource,
    dedup_token: &str,
    transaction_type: TransactionType,
    amount_paise: i64,
) -> AppResult<()> {
    use crate::schema::transactions;

    let reward_exists: bool = diesel::select(diesel::dsl::exists(
        transactions::table
            .filter(transactions::purchase_source.eq(source))
            .filter(transactions::purchase_token.eq(dedup_token)),
    ))
    .get_result(conn)?;

    if !reward_exists {
        let reward = Transaction::new(
            payer_user_id.to_string(),
            transaction_type,
            amount_paise,
            reward_bot_id.to_string(),
            source,
            dedup_token.to_string(),
        );
        diesel::insert_into(transactions::table)
            .values(&reward)
            .execute(conn)?;
    }

    Ok(())
}

/// Newest subscription for (user, bot) that is Active and unexpired.
pub(crate) fn find_active_subscription(
    conn: &mut SqliteConnection,
    user_id_param: &str,
    bot_id_param: &str,
) -> AppResult<Option<BotSubscription>> {
    use crate::schema::bot_subscriptions::dsl::*;

    let now = chrono::Utc::now().naive_utc();

    bot_subscriptions
        .filter(user_id.eq(user_id_param))
        .filter(bot_id.eq(bot_id_param))
        .filter(status.eq(BotSubscriptionStatus::Active))
        .filter(expires_at.gt(now))
        .order(expires_at.desc())
        .first(conn)
        .optional()
        .map_err(AppError::from)
}

/// On plan change/crossgrade Google issues a new purchase token and reports
/// the replaced one as `linkedPurchaseToken` — mark that old row Expired.
fn expire_linked_bot_subscription(
    conn: &mut SqliteConnection,
    linked_purchase_token: Option<&str>,
) -> AppResult<()> {
    use crate::schema::bot_subscriptions::dsl::*;

    if let Some(token) = linked_purchase_token {
        let now = chrono::Utc::now().naive_utc();
        diesel::update(
            bot_subscriptions
                .filter(purchase_source.eq(PurchaseSource::Google))
                .filter(purchase_token.eq(token)),
        )
        .set((
            status.eq(BotSubscriptionStatus::Expired),
            updated_at.eq(now),
        ))
        .execute(conn)?;
    }

    Ok(())
}

#[utoipa::path(
    post,
    path = "/google/bot-subscription/verify",
    request_body = VerifyBotSubscriptionRequest,
    responses(
        (status = 200, description = "Bot subscription verified and access recorded", body = ApiResponse<EmptyData>),
        (status = 400, description = "Invalid, inactive, or already-used purchase token", body = ApiResponse<EmptyData>),
        (status = 500, description = "Internal server error", body = ApiResponse<EmptyData>)
    ),
    tag = "Bot Subscription"
)]
pub async fn verify_google_bot_subscription(
    State(app_state): State<AppState>,
    Json(payload): Json<VerifyBotSubscriptionRequest>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = app_state.get_db_connection()?;

    process_verify_google_bot_subscription(&mut conn, &app_state, &payload).await?;

    Ok((
        StatusCode::OK,
        Json(ApiResponse::<EmptyData>::success(EmptyData {})),
    ))
}

async fn process_verify_google_bot_subscription(
    conn: &mut SqliteConnection,
    app_state: &AppState,
    payload: &VerifyBotSubscriptionRequest,
) -> AppResult<()> {
    use crate::schema::bot_subscriptions::dsl::*;

    if !is_bot_subscription_product(&payload.product_id) {
        return Err(AppError::BadRequest(format!(
            "Invalid product id for bot subscription: {}",
            payload.product_id
        )));
    }

    let existing: Option<BotSubscription> = bot_subscriptions
        .filter(purchase_source.eq(PurchaseSource::Google))
        .filter(purchase_token.eq(&payload.purchase_token))
        .first(conn)
        .optional()?;

    match existing {
        // ── No row yet: verify with the store, acknowledge, reward, insert ──
        // The reward is inserted before the row so a crash at any point
        // converges on retry: the reward dedups on latestOrderId and the
        // acknowledge call skips when already acknowledged.
        None => {
            let subscription_response = fetch_google_play_purchase_details(
                &payload.package_name,
                &payload.purchase_token,
                app_state.google_auth.as_ref(),
            )
            .await?;

            let store_expires_at =
                parse_google_expiry(&subscription_response, &payload.product_id)?;
            verify_subcription_response_for_active_status(&subscription_response)?;

            let user_id_str = subscription_response
                .external_account_identifiers
                .as_ref()
                .and_then(|ids| ids.obfuscated_external_account_id.clone())
                .ok_or(AppError::ExternalAccountIdentifiersMissing)?;

            acknowledge_google_play(
                &payload.package_name,
                &payload.purchase_token,
                &subscription_response,
                app_state.google_auth.as_ref(),
            )
            .await?;

            expire_linked_bot_subscription(
                conn,
                subscription_response.linked_purchase_token.as_deref(),
            )?;

            let reward_dedup_token = subscription_response
                .latest_order_id
                .as_deref()
                .unwrap_or(&payload.purchase_token);
            insert_subscription_reward_if_new(
                conn,
                &user_id_str,
                &payload.bot_id,
                PurchaseSource::Google,
                reward_dedup_token,
                TransactionType::BotSubscriptionInitialReward,
                BOT_SUBSCRIPTION_INITIAL_REWARD_PAISE,
            )?;

            let new_subscription = BotSubscription::new(
                PurchaseSource::Google,
                payload.purchase_token.clone(),
                user_id_str,
                payload.bot_id.clone(),
                payload.product_id.clone(),
                store_expires_at,
            );
            diesel::insert_into(bot_subscriptions)
                .values(&new_subscription)
                .execute(conn)?;

            Ok(())
        }

        // ── Token reused for a different bot: always reject ──
        Some(sub) if sub.bot_id != payload.bot_id => Err(AppError::TokenAlreadyUsed),

        // ── Refunded/revoked: terminal ──
        Some(sub) if sub.status == BotSubscriptionStatus::Canceled => {
            Err(AppError::TokenAlreadyUsed)
        }

        // ── Live and unexpired: idempotent success ──
        Some(sub)
            if sub.status == BotSubscriptionStatus::Active
                && sub.expires_at > chrono::Utc::now().naive_utc() =>
        {
            Ok(())
        }

        // ── Stale row (lapsed, on hold, or expired): re-check with the store.
        // Covers recovery/renewal the RTDN webhook may have missed. ──
        Some(sub) => {
            let subscription_response = fetch_google_play_purchase_details(
                &payload.package_name,
                &payload.purchase_token,
                app_state.google_auth.as_ref(),
            )
            .await?;

            let store_expires_at = parse_google_expiry(&subscription_response, &sub.product_id)?;
            verify_subcription_response_for_active_status(&subscription_response)?;

            let now = chrono::Utc::now().naive_utc();
            diesel::update(bot_subscriptions.filter(id.eq(&sub.id)))
                .set((
                    status.eq(BotSubscriptionStatus::Active),
                    expires_at.eq(store_expires_at),
                    updated_at.eq(now),
                ))
                .execute(conn)?;

            // A renewal charge happened while we weren't looking — pay it out.
            if store_expires_at > sub.expires_at {
                if let Some(order_id) = subscription_response.latest_order_id.as_deref() {
                    insert_subscription_reward_if_new(
                        conn,
                        &sub.user_id,
                        &sub.bot_id,
                        PurchaseSource::Google,
                        order_id,
                        TransactionType::BotSubscriptionRenewalReward,
                        BOT_SUBSCRIPTION_RENEWAL_REWARD_PAISE,
                    )?;
                }
            }

            Ok(())
        }
    }
}

#[utoipa::path(
    post,
    path = "/apple/bot-subscription/grant",
    request_body = GrantAppleBotSubscriptionRequest,
    responses(
        (status = 200, description = "iOS bot subscription recorded", body = ApiResponse<EmptyData>),
        (status = 400, description = "Invalid or already-used Apple transaction", body = ApiResponse<EmptyData>),
        (status = 500, description = "Internal server error", body = ApiResponse<EmptyData>)
    ),
    tag = "Bot Subscription"
)]
pub async fn grant_apple_bot_subscription(
    State(app_state): State<AppState>,
    Json(payload): Json<GrantAppleBotSubscriptionRequest>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = app_state.get_db_connection()?;

    process_grant_apple_bot_subscription(&mut conn, &payload).await?;

    Ok((
        StatusCode::OK,
        Json(ApiResponse::<EmptyData>::success(EmptyData {})),
    ))
}

async fn process_grant_apple_bot_subscription(
    conn: &mut SqliteConnection,
    payload: &GrantAppleBotSubscriptionRequest,
) -> AppResult<()> {
    use crate::schema::bot_subscriptions::dsl::*;

    if !is_bot_subscription_product(&payload.product_id) {
        return Err(AppError::BadRequest(format!(
            "Invalid product id for bot subscription: {}",
            payload.product_id
        )));
    }

    let transaction = fetch_apple_transaction_info(
        &payload.transaction_id,
        &payload.product_id,
        payload
            .environment
            .unwrap_or_else(default_apple_environment),
    )
    .await?;

    // originalTransactionId is the stable subscription key across renewals.
    let subscription_key = transaction
        .original_transaction_id
        .clone()
        .unwrap_or_else(|| transaction.transaction_id.clone());
    let store_expires_at = apple_expiry(&transaction)?;

    let existing: Option<BotSubscription> = bot_subscriptions
        .filter(purchase_source.eq(PurchaseSource::Apple))
        .filter(purchase_token.eq(&subscription_key))
        .first(conn)
        .optional()?;

    match existing {
        // ── No row yet: resolve the user, reward, insert.
        // Reward before row so a crashed attempt converges on retry. ──
        None => {
            let app_account_token = transaction
                .app_account_token
                .as_deref()
                .ok_or(AppError::ExternalAccountIdentifiersMissing)?;
            let user_id_str = resolve_app_account_token(conn, app_account_token)?;

            insert_subscription_reward_if_new(
                conn,
                &user_id_str,
                &payload.bot_id,
                PurchaseSource::Apple,
                &transaction.transaction_id,
                TransactionType::BotSubscriptionInitialReward,
                BOT_SUBSCRIPTION_INITIAL_REWARD_PAISE,
            )?;

            let new_subscription = BotSubscription::new(
                PurchaseSource::Apple,
                subscription_key,
                user_id_str,
                payload.bot_id.clone(),
                payload.product_id.clone(),
                store_expires_at,
            );
            diesel::insert_into(bot_subscriptions)
                .values(&new_subscription)
                .execute(conn)?;

            Ok(())
        }

        // ── Subscription reused for a different bot: always reject ──
        Some(sub) if sub.bot_id != payload.bot_id => Err(AppError::TokenAlreadyUsed),

        // ── Refunded/revoked: terminal ──
        Some(sub) if sub.status == BotSubscriptionStatus::Canceled => {
            Err(AppError::TokenAlreadyUsed)
        }

        // ── Same subscription: refresh if the store expiry advanced.
        // A newer transactionId means a renewal charge we may have missed. ──
        Some(sub) => {
            let now = chrono::Utc::now().naive_utc();

            if store_expires_at > sub.expires_at {
                diesel::update(bot_subscriptions.filter(id.eq(&sub.id)))
                    .set((
                        status.eq(BotSubscriptionStatus::Active),
                        expires_at.eq(store_expires_at),
                        updated_at.eq(now),
                    ))
                    .execute(conn)?;

                insert_subscription_reward_if_new(
                    conn,
                    &sub.user_id,
                    &sub.bot_id,
                    PurchaseSource::Apple,
                    &transaction.transaction_id,
                    TransactionType::BotSubscriptionRenewalReward,
                    BOT_SUBSCRIPTION_RENEWAL_REWARD_PAISE,
                )?;
            } else if sub.status != BotSubscriptionStatus::Active && store_expires_at > now {
                // Same period re-presented (e.g. billing recovered) — reactivate.
                diesel::update(bot_subscriptions.filter(id.eq(&sub.id)))
                    .set((status.eq(BotSubscriptionStatus::Active), updated_at.eq(now)))
                    .execute(conn)?;
            }

            Ok(())
        }
    }
}

#[utoipa::path(
    get,
    path = "/bot-subscription/check",
    params(
        ("user_id" = String, Query, description = "User ID to check the subscription for"),
        ("bot_id" = String, Query, description = "Bot ID to check the subscription for"),
    ),
    responses(
        (status = 200, description = "Subscription check result", body = ApiResponse<BotSubscriptionCheckResponse>),
        (status = 500, description = "Internal server error", body = ApiResponse<EmptyData>)
    ),
    tag = "Bot Subscription"
)]
pub async fn check_bot_subscription(
    State(app_state): State<AppState>,
    Query(params): Query<CheckBotSubscriptionQuery>,
) -> Result<impl IntoResponse, AppError> {
    use crate::schema::bot_subscriptions::dsl::*;

    let mut conn = app_state.get_db_connection()?;

    let newest: Option<BotSubscription> = bot_subscriptions
        .filter(user_id.eq(&params.user_id))
        .filter(bot_id.eq(&params.bot_id))
        .order(expires_at.desc())
        .first(&mut conn)
        .optional()?;

    let now = chrono::Utc::now().naive_utc();
    let response = match newest {
        Some(sub) => BotSubscriptionCheckResponse {
            subscribed: sub.status == BotSubscriptionStatus::Active && sub.expires_at > now,
            status: Some(sub.status),
            expires_at: Some(
                chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                    sub.expires_at,
                    chrono::Utc,
                )
                .to_rfc3339(),
            ),
        },
        None => BotSubscriptionCheckResponse {
            subscribed: false,
            status: None,
            expires_at: None,
        },
    };

    Ok((StatusCode::OK, Json(ApiResponse::success(response))))
}

/// Applies a Google RTDN subscription notification to `bot_subscriptions`.
///
/// Called from the RTDN webhook once the subscription product matches the
/// bot-subscription prefix. Keys off the purchase token; the RTDN payload has
/// no bot_id, so a PURCHASED for an unknown token is a no-op — the client's
/// verify call creates the row (same stance as one-time products).
pub(crate) fn apply_bot_subscription_rtdn(
    conn: &mut SqliteConnection,
    notification_type: i32,
    purchase_token_param: &str,
    response: &GooglePlaySubscriptionResponse,
) -> AppResult<()> {
    use crate::schema::bot_subscriptions::dsl::*;

    expire_linked_bot_subscription(conn, response.linked_purchase_token.as_deref())?;

    let existing: Option<BotSubscription> = bot_subscriptions
        .filter(purchase_source.eq(PurchaseSource::Google))
        .filter(purchase_token.eq(purchase_token_param))
        .first(conn)
        .optional()?;

    let now = chrono::Utc::now().naive_utc();

    match notification_type {
        subscription_notification_type::SUBSCRIPTION_PURCHASED => {
            let Some(sub) = existing else {
                println!("Bot subscription purchased, waiting for client to call verify endpoint");
                return Ok(());
            };
            let store_expires_at = parse_google_expiry(response, &sub.product_id)?;
            diesel::update(bot_subscriptions.filter(id.eq(&sub.id)))
                .set((
                    status.eq(BotSubscriptionStatus::Active),
                    expires_at.eq(store_expires_at),
                    updated_at.eq(now),
                ))
                .execute(conn)?;
            Ok(())
        }

        subscription_notification_type::SUBSCRIPTION_RENEWED
        | subscription_notification_type::SUBSCRIPTION_RECOVERED => {
            // Erroring makes Pub/Sub retry until the client's verify call
            // has created the row.
            let sub = existing.ok_or(AppError::SubscriptionInvalidLineItems)?;
            let store_expires_at = parse_google_expiry(response, &sub.product_id)?;

            diesel::update(bot_subscriptions.filter(id.eq(&sub.id)))
                .set((
                    status.eq(BotSubscriptionStatus::Active),
                    expires_at.eq(store_expires_at.max(sub.expires_at)),
                    updated_at.eq(now),
                ))
                .execute(conn)?;

            // Reward only when the paid period actually advanced; the order id
            // (new per renewal) dedups Pub/Sub redeliveries. Missing order id
            // should not happen — skip rather than risk a double payout.
            if store_expires_at > sub.expires_at {
                match response.latest_order_id.as_deref() {
                    Some(order_id) => insert_subscription_reward_if_new(
                        conn,
                        &sub.user_id,
                        &sub.bot_id,
                        PurchaseSource::Google,
                        order_id,
                        TransactionType::BotSubscriptionRenewalReward,
                        BOT_SUBSCRIPTION_RENEWAL_REWARD_PAISE,
                    )?,
                    None => println!(
                        "Bot subscription renewal without latestOrderId; skipping reward for token {}",
                        purchase_token_param
                    ),
                }
            }
            Ok(())
        }

        subscription_notification_type::SUBSCRIPTION_IN_GRACE_PERIOD
        | subscription_notification_type::SUBSCRIPTION_RESTARTED => {
            // User keeps access; the store extends/keeps the expiry.
            if let Some(sub) = existing {
                let store_expires_at = parse_google_expiry(response, &sub.product_id)?;
                diesel::update(bot_subscriptions.filter(id.eq(&sub.id)))
                    .set((
                        status.eq(BotSubscriptionStatus::Active),
                        expires_at.eq(store_expires_at.max(sub.expires_at)),
                        updated_at.eq(now),
                    ))
                    .execute(conn)?;
            }
            Ok(())
        }

        subscription_notification_type::SUBSCRIPTION_ON_HOLD => {
            if let Some(sub) = existing {
                diesel::update(bot_subscriptions.filter(id.eq(&sub.id)))
                    .set((status.eq(BotSubscriptionStatus::OnHold), updated_at.eq(now)))
                    .execute(conn)?;
            }
            Ok(())
        }

        subscription_notification_type::SUBSCRIPTION_REVOKED => {
            if let Some(sub) = existing {
                diesel::update(bot_subscriptions.filter(id.eq(&sub.id)))
                    .set((
                        status.eq(BotSubscriptionStatus::Canceled),
                        updated_at.eq(now),
                    ))
                    .execute(conn)?;
            }
            Ok(())
        }

        subscription_notification_type::SUBSCRIPTION_EXPIRED => {
            if let Some(sub) = existing {
                diesel::update(bot_subscriptions.filter(id.eq(&sub.id)))
                    .set((
                        status.eq(BotSubscriptionStatus::Expired),
                        updated_at.eq(now),
                    ))
                    .execute(conn)?;
            }
            Ok(())
        }

        // CANCELED = auto-renew turned off; access continues until expiry.
        // Price change / deferred / pause events need no state change either.
        _ => {
            println!(
                "Bot subscription notification type {} for token {}: no action",
                notification_type, purchase_token_param
            );
            Ok(())
        }
    }
}

/// Applies an Apple App Store server notification to `bot_subscriptions`.
///
/// Keyed by originalTransactionId (falling back to transactionId), which is
/// stable across renewals. An unknown key is a no-op: the notification has no
/// bot_id, so the client's grant call creates the row.
pub(crate) fn process_apple_bot_subscription_notification(
    conn: &mut SqliteConnection,
    notification: &AppleNotificationDecodedPayload,
    transaction: &AppleJWSTransactionDecodedPayload,
) -> AppResult<()> {
    use crate::schema::bot_subscriptions::dsl::*;

    let subscription_key = transaction
        .original_transaction_id
        .clone()
        .unwrap_or_else(|| transaction.transaction_id.clone());

    let existing: Option<BotSubscription> = bot_subscriptions
        .filter(purchase_source.eq(PurchaseSource::Apple))
        .filter(purchase_token.eq(&subscription_key))
        .first(conn)
        .optional()?;

    let Some(sub) = existing else {
        println!(
            "Apple bot subscription notification {} for unknown subscription {}; waiting for client grant",
            notification.notification_type, subscription_key
        );
        return Ok(());
    };

    let now = chrono::Utc::now().naive_utc();

    match notification.notification_type.as_str() {
        "SUBSCRIBED" => {
            let store_expires_at = apple_expiry(transaction)?;
            diesel::update(bot_subscriptions.filter(id.eq(&sub.id)))
                .set((
                    status.eq(BotSubscriptionStatus::Active),
                    expires_at.eq(store_expires_at.max(sub.expires_at)),
                    updated_at.eq(now),
                ))
                .execute(conn)?;
            Ok(())
        }

        "DID_RENEW" => {
            let store_expires_at = apple_expiry(transaction)?;
            diesel::update(bot_subscriptions.filter(id.eq(&sub.id)))
                .set((
                    status.eq(BotSubscriptionStatus::Active),
                    expires_at.eq(store_expires_at.max(sub.expires_at)),
                    updated_at.eq(now),
                ))
                .execute(conn)?;

            // Each Apple renewal carries a fresh transactionId — the natural
            // dedup key. The expiry-advance gate catches replayed payloads.
            if store_expires_at > sub.expires_at {
                insert_subscription_reward_if_new(
                    conn,
                    &sub.user_id,
                    &sub.bot_id,
                    PurchaseSource::Apple,
                    &transaction.transaction_id,
                    TransactionType::BotSubscriptionRenewalReward,
                    BOT_SUBSCRIPTION_RENEWAL_REWARD_PAISE,
                )?;
            }
            Ok(())
        }

        "DID_FAIL_TO_RENEW" => {
            // With a grace period the user keeps access (expiresDate governs);
            // without one, access is on hold until billing recovers.
            if notification.subtype.as_deref() != Some("GRACE_PERIOD") {
                diesel::update(bot_subscriptions.filter(id.eq(&sub.id)))
                    .set((status.eq(BotSubscriptionStatus::OnHold), updated_at.eq(now)))
                    .execute(conn)?;
            }
            Ok(())
        }

        "GRACE_PERIOD_EXPIRED" => {
            diesel::update(bot_subscriptions.filter(id.eq(&sub.id)))
                .set((status.eq(BotSubscriptionStatus::OnHold), updated_at.eq(now)))
                .execute(conn)?;
            Ok(())
        }

        "EXPIRED" => {
            diesel::update(bot_subscriptions.filter(id.eq(&sub.id)))
                .set((
                    status.eq(BotSubscriptionStatus::Expired),
                    updated_at.eq(now),
                ))
                .execute(conn)?;
            Ok(())
        }

        "REFUND" | "REVOKE" => {
            diesel::update(bot_subscriptions.filter(id.eq(&sub.id)))
                .set((
                    status.eq(BotSubscriptionStatus::Canceled),
                    updated_at.eq(now),
                ))
                .execute(conn)?;
            Ok(())
        }

        // DID_CHANGE_RENEWAL_STATUS = auto-renew toggled; access until expiry.
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SubscriptionLineItem;
    use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

    const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

    struct TestDb {
        path: String,
        conn: SqliteConnection,
    }

    impl TestDb {
        fn new() -> Self {
            let path = format!("./test_bot_sub_rtdn_{}.db", uuid::Uuid::new_v4());
            let mut conn = SqliteConnection::establish(&path).unwrap();
            conn.run_pending_migrations(MIGRATIONS).unwrap();
            Self { path, conn }
        }
    }

    impl Drop for TestDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn store_response(
        expiry: chrono::DateTime<chrono::Utc>,
        latest_order_id: Option<&str>,
        linked_purchase_token: Option<&str>,
    ) -> GooglePlaySubscriptionResponse {
        GooglePlaySubscriptionResponse {
            kind: "androidpublisher#subscriptionPurchaseV2".to_string(),
            start_time: None,
            region_code: None,
            subscription_state: "SUBSCRIPTION_STATE_ACTIVE".to_string(),
            latest_order_id: latest_order_id.map(str::to_string),
            acknowledgement_state: "ACKNOWLEDGEMENT_STATE_ACKNOWLEDGED".to_string(),
            line_items: vec![SubscriptionLineItem {
                product_id: "bot_sub_test".to_string(),
                expiry_time: Some(expiry.to_rfc3339()),
                auto_renewing: Some(true),
                price_change_state: None,
            }],
            linked_purchase_token: linked_purchase_token.map(str::to_string),
            external_account_identifiers: None,
            subscribe_with_google_info: None,
        }
    }

    fn insert_subscription(
        conn: &mut SqliteConnection,
        token: &str,
        sub_status: BotSubscriptionStatus,
        expires: chrono::DateTime<chrono::Utc>,
    ) -> BotSubscription {
        let mut sub = BotSubscription::new(
            PurchaseSource::Google,
            token.to_string(),
            "user-1".to_string(),
            "bot-1".to_string(),
            "bot_sub_test".to_string(),
            expires.naive_utc(),
        );
        sub.status = sub_status;
        diesel::insert_into(crate::schema::bot_subscriptions::table)
            .values(&sub)
            .execute(conn)
            .unwrap();
        sub
    }

    fn load_subscription(conn: &mut SqliteConnection, token: &str) -> BotSubscription {
        use crate::schema::bot_subscriptions::dsl::*;
        bot_subscriptions
            .filter(purchase_token.eq(token))
            .first(conn)
            .unwrap()
    }

    fn renewal_reward_count(conn: &mut SqliteConnection) -> i64 {
        use crate::schema::transactions::dsl::*;
        transactions
            .filter(transaction_type.eq(TransactionType::BotSubscriptionRenewalReward))
            .count()
            .get_result(conn)
            .unwrap()
    }

    #[test]
    fn renewed_advances_expiry_and_pays_once() {
        let mut db = TestDb::new();
        let old_expiry = chrono::Utc::now() + chrono::Duration::days(1);
        let new_expiry = chrono::Utc::now() + chrono::Duration::days(8);
        insert_subscription(
            &mut db.conn,
            "tok-1",
            BotSubscriptionStatus::Active,
            old_expiry,
        );

        let response = store_response(new_expiry, Some("GPA.1234..1"), None);
        apply_bot_subscription_rtdn(
            &mut db.conn,
            subscription_notification_type::SUBSCRIPTION_RENEWED,
            "tok-1",
            &response,
        )
        .unwrap();

        let sub = load_subscription(&mut db.conn, "tok-1");
        assert_eq!(sub.status, BotSubscriptionStatus::Active);
        assert_eq!(sub.expires_at, new_expiry.naive_utc());
        assert_eq!(renewal_reward_count(&mut db.conn), 1);

        // Pub/Sub redelivery of the same notification: expiry unchanged, no new reward
        apply_bot_subscription_rtdn(
            &mut db.conn,
            subscription_notification_type::SUBSCRIPTION_RENEWED,
            "tok-1",
            &response,
        )
        .unwrap();
        assert_eq!(renewal_reward_count(&mut db.conn), 1);

        // A later renewal with a fresh order id pays again
        let next_expiry = chrono::Utc::now() + chrono::Duration::days(15);
        let response = store_response(next_expiry, Some("GPA.1234..2"), None);
        apply_bot_subscription_rtdn(
            &mut db.conn,
            subscription_notification_type::SUBSCRIPTION_RENEWED,
            "tok-1",
            &response,
        )
        .unwrap();
        assert_eq!(renewal_reward_count(&mut db.conn), 2);
    }

    #[test]
    fn renewed_without_order_id_skips_reward_but_updates_expiry() {
        let mut db = TestDb::new();
        let old_expiry = chrono::Utc::now() + chrono::Duration::days(1);
        let new_expiry = chrono::Utc::now() + chrono::Duration::days(8);
        insert_subscription(
            &mut db.conn,
            "tok-1",
            BotSubscriptionStatus::Active,
            old_expiry,
        );

        let response = store_response(new_expiry, None, None);
        apply_bot_subscription_rtdn(
            &mut db.conn,
            subscription_notification_type::SUBSCRIPTION_RENEWED,
            "tok-1",
            &response,
        )
        .unwrap();

        let sub = load_subscription(&mut db.conn, "tok-1");
        assert_eq!(sub.expires_at, new_expiry.naive_utc());
        assert_eq!(renewal_reward_count(&mut db.conn), 0);
    }

    #[test]
    fn renewed_for_unknown_token_errors_for_pubsub_retry() {
        let mut db = TestDb::new();
        let response = store_response(chrono::Utc::now() + chrono::Duration::days(7), None, None);

        let result = apply_bot_subscription_rtdn(
            &mut db.conn,
            subscription_notification_type::SUBSCRIPTION_RENEWED,
            "unknown-token",
            &response,
        );
        assert!(result.is_err());
    }

    #[test]
    fn purchased_for_unknown_token_is_noop() {
        let mut db = TestDb::new();
        let response = store_response(chrono::Utc::now() + chrono::Duration::days(7), None, None);

        apply_bot_subscription_rtdn(
            &mut db.conn,
            subscription_notification_type::SUBSCRIPTION_PURCHASED,
            "unknown-token",
            &response,
        )
        .unwrap();
    }

    #[test]
    fn recovered_reactivates_on_hold_subscription() {
        let mut db = TestDb::new();
        let old_expiry = chrono::Utc::now() - chrono::Duration::days(1);
        let new_expiry = chrono::Utc::now() + chrono::Duration::days(7);
        insert_subscription(
            &mut db.conn,
            "tok-1",
            BotSubscriptionStatus::OnHold,
            old_expiry,
        );

        let response = store_response(new_expiry, Some("GPA.1234..3"), None);
        apply_bot_subscription_rtdn(
            &mut db.conn,
            subscription_notification_type::SUBSCRIPTION_RECOVERED,
            "tok-1",
            &response,
        )
        .unwrap();

        let sub = load_subscription(&mut db.conn, "tok-1");
        assert_eq!(sub.status, BotSubscriptionStatus::Active);
        assert_eq!(sub.expires_at, new_expiry.naive_utc());
        assert_eq!(renewal_reward_count(&mut db.conn), 1);
    }

    #[test]
    fn lifecycle_status_transitions() {
        let mut db = TestDb::new();
        let expiry = chrono::Utc::now() + chrono::Duration::days(7);
        let response = store_response(expiry, None, None);

        insert_subscription(
            &mut db.conn,
            "tok-hold",
            BotSubscriptionStatus::Active,
            expiry,
        );
        apply_bot_subscription_rtdn(
            &mut db.conn,
            subscription_notification_type::SUBSCRIPTION_ON_HOLD,
            "tok-hold",
            &response,
        )
        .unwrap();
        assert_eq!(
            load_subscription(&mut db.conn, "tok-hold").status,
            BotSubscriptionStatus::OnHold
        );

        insert_subscription(
            &mut db.conn,
            "tok-rev",
            BotSubscriptionStatus::Active,
            expiry,
        );
        apply_bot_subscription_rtdn(
            &mut db.conn,
            subscription_notification_type::SUBSCRIPTION_REVOKED,
            "tok-rev",
            &response,
        )
        .unwrap();
        assert_eq!(
            load_subscription(&mut db.conn, "tok-rev").status,
            BotSubscriptionStatus::Canceled
        );

        insert_subscription(
            &mut db.conn,
            "tok-exp",
            BotSubscriptionStatus::Active,
            expiry,
        );
        apply_bot_subscription_rtdn(
            &mut db.conn,
            subscription_notification_type::SUBSCRIPTION_EXPIRED,
            "tok-exp",
            &response,
        )
        .unwrap();
        assert_eq!(
            load_subscription(&mut db.conn, "tok-exp").status,
            BotSubscriptionStatus::Expired
        );

        // CANCELED (auto-renew off) leaves the row untouched
        insert_subscription(
            &mut db.conn,
            "tok-can",
            BotSubscriptionStatus::Active,
            expiry,
        );
        apply_bot_subscription_rtdn(
            &mut db.conn,
            subscription_notification_type::SUBSCRIPTION_CANCELED,
            "tok-can",
            &response,
        )
        .unwrap();
        assert_eq!(
            load_subscription(&mut db.conn, "tok-can").status,
            BotSubscriptionStatus::Active
        );
    }

    #[test]
    fn linked_purchase_token_expires_replaced_subscription() {
        let mut db = TestDb::new();
        let expiry = chrono::Utc::now() + chrono::Duration::days(7);
        insert_subscription(
            &mut db.conn,
            "old-tok",
            BotSubscriptionStatus::Active,
            expiry,
        );
        insert_subscription(
            &mut db.conn,
            "new-tok",
            BotSubscriptionStatus::Active,
            expiry,
        );

        let response = store_response(expiry, None, Some("old-tok"));
        apply_bot_subscription_rtdn(
            &mut db.conn,
            subscription_notification_type::SUBSCRIPTION_PURCHASED,
            "new-tok",
            &response,
        )
        .unwrap();

        assert_eq!(
            load_subscription(&mut db.conn, "old-tok").status,
            BotSubscriptionStatus::Expired
        );
        assert_eq!(
            load_subscription(&mut db.conn, "new-tok").status,
            BotSubscriptionStatus::Active
        );
    }
}
