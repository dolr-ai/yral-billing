use crate::error::AppError;
use crate::model::Transaction;
use crate::types::{ApiResponse, BalanceResponse, TransactionResponse};
use crate::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use diesel::prelude::*;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct UserTransactionsQuery {
    pub recipient_id: String,
}

#[derive(Deserialize)]
pub struct BalanceQuery {
    pub recipient_id: String,
}

#[utoipa::path(
    get,
    path = "/transactions",
    params(
        ("recipient_id" = String, Query, description = "Recipient ID to fetch transactions for"),
    ),
    responses(
        (status = 200, description = "List of transactions for the user", body = ApiResponse<Vec<TransactionResponse>>),
        (status = 500, description = "Internal server error", body = ApiResponse<Vec<TransactionResponse>>)
    ),
    tag = "Transactions"
)]
pub async fn get_user_transactions(
    State(app_state): State<AppState>,
    Query(params): Query<UserTransactionsQuery>,
) -> Result<impl IntoResponse, AppError> {
    use crate::schema::transactions::dsl::*;

    let mut conn = app_state.get_db_connection()?;

    let rows: Vec<Transaction> = transactions
        .filter(recipient_id.eq(&params.recipient_id))
        .order(created_at.desc())
        .load(&mut conn)?;

    let response: Vec<TransactionResponse> = rows
        .into_iter()
        .map(|t| TransactionResponse {
            id: t.id,
            user_id: t.user_id,
            transaction_type: t.transaction_type,
            amount_paise: t.amount_paise,
            recipient_id: t.recipient_id,
            purchase_token: t.purchase_token,
            created_at: chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                t.created_at,
                chrono::Utc,
            )
            .to_rfc3339(),
        })
        .collect();

    Ok((StatusCode::OK, Json(ApiResponse::success(response))))
}

#[utoipa::path(
    get,
    path = "/transactions/balance",
    params(
        ("recipient_id" = String, Query, description = "Bot ID to fetch accumulated reward balance for"),
    ),
    responses(
        (status = 200, description = "Current reward balance for the recipient", body = ApiResponse<BalanceResponse>),
        (status = 500, description = "Internal server error", body = ApiResponse<BalanceResponse>)
    ),
    tag = "Transactions"
)]
pub async fn get_balance(
    State(app_state): State<AppState>,
    Query(params): Query<BalanceQuery>,
) -> Result<impl IntoResponse, AppError> {
    use crate::schema::transactions::dsl::*;

    let mut conn = app_state.get_db_connection()?;

    let total: Option<i64> = transactions
        .filter(recipient_id.eq(&params.recipient_id))
        .select(diesel::dsl::sql::<
            diesel::sql_types::Nullable<diesel::sql_types::BigInt>,
        >("SUM(amount_paise)"))
        .first(&mut conn)?;

    let balance = total.unwrap_or(0);

    Ok((
        StatusCode::OK,
        Json(ApiResponse::success(BalanceResponse {
            balance_paise: balance,
            balance_rupees: balance as f64 / 100.0,
        })),
    ))
}
