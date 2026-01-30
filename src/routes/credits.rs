use axum::{extract::State, Json};
use ic_agent::export::Principal;
use yral_canisters_client::{ic::USER_INFO_SERVICE_ID, user_info_service::UserInfoService};

use crate::{
    error::AppError,
    types::{ApiResponse, CreditRequest, EmptyData},
    AppState,
};

/// Deduct credits from a user's account
///
/// Requires JWT authentication in Authorization header
#[utoipa::path(
    post,
    path = "/credits/deduct",
    request_body = CreditRequest,
    responses(
        (status = 200, description = "Credits deducted successfully", body = ApiResponse<EmptyData>),
        (status = 400, description = "Invalid request", body = ApiResponse<EmptyData>),
        (status = 401, description = "Unauthorized - Invalid or missing JWT token"),
        (status = 500, description = "Internal server error", body = ApiResponse<EmptyData>)
    ),
    tag = "Credits",
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn deduct_credits(
    State(state): State<AppState>,
    Json(payload): Json<CreditRequest>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    // Get IC agent
    let admin_ic_agent = state
        .admin_ic_agent
        .as_ref()
        .ok_or(AppError::AdminIcAgentMissing)?;

    // Parse user principal
    let user_principal = Principal::from_text(&payload.user_principal)
        .map_err(|e| AppError::BadRequest(format!("Invalid user principal: {}", e)))?;

    // Create user info service client
    let user_info_client = UserInfoService(USER_INFO_SERVICE_ID, admin_ic_agent);

    // Call canister to deduct credits
    let result = user_info_client
        .remove_pro_plan_free_video_credits(user_principal, payload.amount)
        .await
        .map_err(|e| AppError::NetworkError(format!("Failed to deduct credits: {}", e)))?;

    // Check canister result
    match result {
        yral_canisters_client::user_info_service::Result_::Ok => {
            Ok(Json(ApiResponse::ok_with_msg(format!(
                "Successfully deducted {} credits from user",
                payload.amount
            ))))
        }
        yral_canisters_client::user_info_service::Result_::Err(e) => Err(AppError::BadRequest(
            format!("Canister returned error: {}", e),
        )),
    }
}

/// Increment credits to a user's account
///
/// Requires JWT authentication in Authorization header
#[utoipa::path(
    post,
    path = "/credits/increment",
    request_body = CreditRequest,
    responses(
        (status = 200, description = "Credits incremented successfully", body = ApiResponse<EmptyData>),
        (status = 400, description = "Invalid request", body = ApiResponse<EmptyData>),
        (status = 401, description = "Unauthorized - Invalid or missing JWT token"),
        (status = 500, description = "Internal server error", body = ApiResponse<EmptyData>)
    ),
    tag = "Credits",
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn increment_credits(
    State(state): State<AppState>,
    Json(payload): Json<CreditRequest>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    // Get IC agent
    let admin_ic_agent = state
        .admin_ic_agent
        .as_ref()
        .ok_or(AppError::AdminIcAgentMissing)?;

    // Parse user principal
    let user_principal = Principal::from_text(&payload.user_principal)
        .map_err(|e| AppError::BadRequest(format!("Invalid user principal: {}", e)))?;

    // Create user info service client
    let user_info_client = UserInfoService(USER_INFO_SERVICE_ID, admin_ic_agent);

    // Call canister to add credits
    let result = user_info_client
        .add_pro_plan_free_video_credits(user_principal, payload.amount)
        .await
        .map_err(|e| AppError::NetworkError(format!("Failed to increment credits: {}", e)))?;

    // Check canister result
    match result {
        yral_canisters_client::user_info_service::Result_::Ok => {
            Ok(Json(ApiResponse::ok_with_msg(format!(
                "Successfully added {} credits to user",
                payload.amount
            ))))
        }
        yral_canisters_client::user_info_service::Result_::Err(e) => Err(AppError::BadRequest(
            format!("Canister returned error: {}", e),
        )),
    }
}
