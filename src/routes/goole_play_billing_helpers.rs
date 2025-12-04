use std::sync::Arc;

use crate::{
    auth::GoogleAuth,
    error::AppResult,
    types::{GooglePlaySubscriptionResponse, VerifyRequest},
};

pub async fn acknowledge_google_play(
    package_name: &str,
    purchase_token: &str,
    subscription_response: &GooglePlaySubscriptionResponse,
    auth: Option<&Arc<GoogleAuth>>,
) -> AppResult<()> {
    // Use mock acknowledgment when local or mock-google-api feature is enabled
    #[cfg(any(feature = "local", feature = "mock-google-api"))]
    {
        let _ = payload; // Suppress unused variable warning
        Ok(())
    }

    #[cfg(not(any(feature = "local", feature = "mock-google-api")))]
    {
        // Get OAuth access token from app state

        if subscription_response.acknowledgement_state != ACKNOWLEDGEMENT_STATE_PENDING {
            return Ok(());
        }

        use crate::{
            error::AppError,
            types::google_play_acknowledgement_state::ACKNOWLEDGEMENT_STATE_PENDING,
        };
        let auth = auth.ok_or(AppError::AuthServiceUnavailable)?;

        let access_token = auth
            .get_token_for_default_scopes()
            .await
            .map_err(|e| AppError::AccessTokenFailed(e.to_string()))?;

        let ack_url = format!(
            "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/purchases/subscriptionsv2/tokens/{}:acknowledge",
            package_name, purchase_token
        );

        let client = reqwest::Client::new();
        let ack_res = client
            .post(&ack_url)
            .bearer_auth(&access_token)
            .send()
            .await
            .map_err(AppError::from)?;

        if ack_res.status().is_success() {
            Ok(())
        } else {
            let error_text = ack_res.text().await.unwrap_or_default();
            Err(AppError::GooglePlayApi(format!(
                "Acknowledgment failed: {}",
                error_text
            )))
        }
    }
}

pub async fn fetch_google_play_purchase_details(
    package_name: &str,
    purchase_token: &str,
    auth: Option<&Arc<GoogleAuth>>,
) -> AppResult<GooglePlaySubscriptionResponse> {
    #[cfg(any(feature = "local", feature = "mock-google-api"))]
    {
        return Ok(GooglePlaySubscriptionResponse {
            kind: "androidpublisher#subscriptionPurchaseV2".to_string(),
            start_time: Some("2023-01-01T00:00:00.000Z".to_string()),
            region_code: Some("US".to_string()),
            subscription_state: google_play_subscription_state::SUBSCRIPTION_STATE_ACTIVE
                .to_string(),
            latest_order_id: Some("GPA.0000-0000-0000-00000".to_string()),
            acknowledgement_state: "ACKNOWLEDGEMENT_STATE_PENDING".to_string(),
            line_items: vec![SubscriptionLineItem {
                product_id: payload.product_id.clone(),
                expiry_time: Some("2024-01-01T00:00:00.000Z".to_string()),
                auto_renewing: Some(true),
                price_change_state: Some("PRICE_CHANGE_STATE_APPLIED".to_string()),
            }],
            linked_purchase_token: None,
            purchase_token: payload.purchase_token.clone(),
        });
    }

    #[cfg(not(any(feature = "local", feature = "mock-google-api")))]
    {
        // Get OAuth access token from app state

        use crate::error::AppError;
        let auth = auth.ok_or(AppError::AuthServiceUnavailable)?;
        let access_token = auth
            .get_token_for_default_scopes()
            .await
            .map_err(|e| AppError::AccessTokenFailed(e.to_string()))?;

        let url = format!(
            "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/purchases/subscriptionsv2/tokens/{}",
            package_name, purchase_token
        );

        let client = reqwest::Client::new();
        let res = client
            .get(&url)
            .bearer_auth(&access_token)
            .send()
            .await
            .map_err(AppError::from)?;

        if res.status().is_success() {
            let subscription_response = res
                .json::<GooglePlaySubscriptionResponse>()
                .await
                .map_err(|e| AppError::GooglePlayResponseParse(e.to_string()))?;

            Ok(subscription_response)
        } else {
            Err(AppError::GooglePlayApi(format!(
                "API returned error status: {}",
                res.status()
            )))
        }
    }
}
