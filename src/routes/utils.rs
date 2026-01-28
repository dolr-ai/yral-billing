use std::sync::Arc;

use ic_agent::export::Principal;
use yral_canisters_client::{
    ic::USER_INFO_SERVICE_ID,
    user_info_service::{SubscriptionPlan, UserInfoService, YralProSubscription},
};

use crate::{
    auth::GoogleAuth, consts::YRAL_PRO_CREDIT_ALLOTMENT, error::AppError, types::VerifyRequest,
};

pub async fn get_valid_google_play_purchase_token_detail(
    payload: &VerifyRequest,
    auth: Option<&Arc<GoogleAuth>>,
) -> Result<serde_json::Value, AppError> {
    // Use mock verification when local or mock-google-api feature is enabled
    #[cfg(any(feature = "local", feature = "mock-google-api"))]
    {
        let _ = payload; // Suppress unused variable warning
        return Ok(serde_json::json!({
            "kind": "androidpublisher#subscriptionPurchaseV2",
            "startTime": "2023-01-01T00:00:00.000Z",
            "regionCode": "US",
            "subscriptionState": "SUBSCRIPTION_STATE_ACTIVE",
            "latestOrderId": "GPA.0000-0000-0000-00000",
            "acknowledgementState": "ACKNOWLEDGEMENT_STATE_PENDING",
            "lineItems": [{
                "productId": payload.product_id,
                "expiryTime": "2024-01-01T00:00:00.000Z",
                "autoRenewing": true,
                "priceChangeState": "PRICE_CHANGE_STATE_APPLIED"
            }],
            "linkedPurchaseToken": null,
            "purchaseToken": payload.purchase_token
        }));
    }

    #[cfg(not(any(feature = "local", feature = "mock-google-api")))]
    {
        // Get OAuth access token from app state
        let auth = auth.ok_or(AppError::AuthServiceUnavailable)?;
        let access_token = auth
            .get_token_for_default_scopes()
            .await
            .map_err(|e| AppError::AccessTokenFailed(e.to_string()))?;

        let url = format!(
            "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/purchases/subscriptionsv2/tokens/{}",
            payload.package_name, payload.purchase_token
        );

        let client = reqwest::Client::new();
        let res = client
            .get(&url)
            .bearer_auth(&access_token)
            .send()
            .await
            .map_err(AppError::from)?;

        if res.status().is_success() {
            let json = res
                .json::<serde_json::Value>()
                .await
                .map_err(|e| AppError::GooglePlayResponseParse(e.to_string()))?;

            // Validate subscription state
            let subscription_state = json.get("subscriptionState");
            if let Some(state) = subscription_state {
                match state.as_str() {
                    Some("SUBSCRIPTION_STATE_ACTIVE") => Ok(json),
                    Some("SUBSCRIPTION_STATE_CANCELED") => Err(AppError::SubscriptionCanceled),
                    Some("SUBSCRIPTION_STATE_IN_GRACE_PERIOD") => Ok(json),
                    Some("SUBSCRIPTION_STATE_ON_HOLD") => Err(AppError::SubscriptionOnHold),
                    Some("SUBSCRIPTION_STATE_PAUSED") => Err(AppError::SubscriptionPaused),
                    Some("SUBSCRIPTION_STATE_EXPIRED") => Err(AppError::SubscriptionExpired),
                    _ => Err(AppError::SubscriptionInvalidState),
                }
            } else {
                Err(AppError::SubscriptionNoState)
            }
        } else {
            Err(AppError::GooglePlayApi(format!(
                "API returned error status: {}",
                res.status()
            )))
        }
    }
}

pub async fn revoke_yral_pro_plan_access(
    admin_ic_agent: &ic_agent::Agent,
    user_id: &str,
) -> Result<(), AppError> {
    let user_info_client = UserInfoService(USER_INFO_SERVICE_ID, admin_ic_agent);
    let user_princpal = Principal::from_text(user_id.to_owned())
        .map_err(|e| AppError::InternalError(e.to_string()))?;

    user_info_client
        .change_subscription_plan(user_princpal, SubscriptionPlan::Free)
        .await
        .map_err(|e| AppError::ServiceAccessFailed(e.to_string()))?;

    Ok(())
}

pub async fn grant_yral_pro_plan_access(
    admin_ic_agent: &ic_agent::Agent,
    user_id: &str,
) -> Result<(), AppError> {
    let user_info_client = UserInfoService(USER_INFO_SERVICE_ID, admin_ic_agent);
    let user_princpal = Principal::from_text(user_id.to_owned())
        .map_err(|e| AppError::InternalError(e.to_string()))?;

    user_info_client
        .change_subscription_plan(
            user_princpal,
            SubscriptionPlan::Pro(YralProSubscription {
                total_video_credits_alloted: YRAL_PRO_CREDIT_ALLOTMENT,
                free_video_credits_left: YRAL_PRO_CREDIT_ALLOTMENT, //default value
            }),
        )
        .await
        .map_err(|e| AppError::ServiceAccessFailed(e.to_string()))?;

    Ok(())
}
