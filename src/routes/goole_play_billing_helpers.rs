use std::sync::Arc;

use crate::{
    auth::GoogleAuth,
    error::AppResult,
    types::{GooglePlayProductPurchaseV2, GooglePlaySubscriptionResponse},
};

#[cfg(feature = "local")]
pub async fn acknowledge_google_play(
    _package_name: &str,
    _purchase_token: &str,
    _subscription_response: &GooglePlaySubscriptionResponse,
    _auth: Option<&Arc<GoogleAuth>>,
) -> AppResult<()> {
    // Mock acknowledgment for local development
    Ok(())
}

#[cfg(not(feature = "local"))]
pub async fn acknowledge_google_play(
    package_name: &str,
    purchase_token: &str,
    subscription_response: &GooglePlaySubscriptionResponse,
    auth: Option<&Arc<GoogleAuth>>,
) -> AppResult<()> {
    // Get OAuth access token from app state

    if subscription_response.acknowledgement_state != ACKNOWLEDGEMENT_STATE_PENDING {
        return Ok(());
    }

    use crate::{
        error::AppError, types::google_play_acknowledgement_state::ACKNOWLEDGEMENT_STATE_PENDING,
    };
    let auth = auth.ok_or(AppError::AuthServiceUnavailable)?;

    let access_token = auth
        .get_token_for_default_scopes()
        .await
        .map_err(|e| AppError::AccessTokenFailed(e.to_string()))?;

    let ack_url = format!(
            "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/purchases/subscriptions/tokens/{}:acknowledge",
            package_name, purchase_token
        );

    let client = reqwest::Client::new();
    let ack_res = client
        .post(&ack_url)
        .bearer_auth(&access_token)
        .header("Content-Type", "application/json")
        .body("{}")
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

#[cfg(feature = "local")]
pub async fn fetch_google_play_purchase_details(
    _package_name: &str,
    _purchase_token: &str,
    _auth: Option<&Arc<GoogleAuth>>,
) -> AppResult<GooglePlaySubscriptionResponse> {
    use crate::types::{
        google_play_subscription_state, ExternalAccountIdentifiers, SubscriptionLineItem,
    };

    return Ok(GooglePlaySubscriptionResponse {
        kind: "androidpublisher#subscriptionPurchaseV2".to_string(),
        start_time: Some("2023-01-01T00:00:00.000Z".to_string()),
        region_code: Some("US".to_string()),
        subscription_state: google_play_subscription_state::SUBSCRIPTION_STATE_ACTIVE.to_string(),
        latest_order_id: Some("GPA.0000-0000-0000-00000".to_string()),
        acknowledgement_state: "ACKNOWLEDGEMENT_STATE_PENDING".to_string(),
        line_items: vec![SubscriptionLineItem {
            product_id: "mock-product-id".to_string(),
            expiry_time: Some("2024-01-01T00:00:00.000Z".to_string()),
            auto_renewing: Some(true),
            price_change_state: Some("PRICE_CHANGE_STATE_APPLIED".to_string()),
        }],
        linked_purchase_token: None,
        external_account_identifiers: Some(ExternalAccountIdentifiers {
            external_account_id: Some("mock-external-account-id".to_string()),
            obfuscated_external_account_id: Some("mock-obfuscated-id".to_string()),
            obfuscated_external_profile_id: Some("mock-obfuscated-profile-id".to_string()),
        }),
        subscribe_with_google_info: None,
    });
}

#[cfg(not(feature = "local"))]
pub async fn fetch_google_play_purchase_details(
    package_name: &str,
    purchase_token: &str,
    auth: Option<&Arc<GoogleAuth>>,
) -> AppResult<GooglePlaySubscriptionResponse> {
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

#[cfg(feature = "local")]
pub async fn fetch_google_play_product_details(
    _package_name: &str,
    _purchase_token: &str,
    _auth: Option<&Arc<GoogleAuth>>,
) -> AppResult<GooglePlayProductPurchaseV2> {
    use crate::types::google_play_product_purchase_state;

    Ok(GooglePlayProductPurchaseV2 {
        kind: Some("androidpublisher#productPurchaseV2".to_string()),
        purchase_time_millis: Some("1700000000000".to_string()),
        purchase_state: google_play_product_purchase_state::PURCHASE_STATE_PURCHASED,
        consumption_state: Some(0),
        acknowledgement_state: Some(0),
        product_id: Some("mock-product-id".to_string()),
        quantity: Some(1),
        obfuscated_external_account_id: Some("mock-user-id".to_string()),
        obfuscated_external_profile_id: None,
        region_code: Some("US".to_string()),
    })
}

#[cfg(not(feature = "local"))]
pub async fn fetch_google_play_product_details(
    package_name: &str,
    purchase_token: &str,
    auth: Option<&Arc<GoogleAuth>>,
) -> AppResult<GooglePlayProductPurchaseV2> {
    use crate::error::AppError;

    let auth = auth.ok_or(AppError::AuthServiceUnavailable)?;
    let access_token = auth
        .get_token_for_default_scopes()
        .await
        .map_err(|e| AppError::AccessTokenFailed(e.to_string()))?;

    let url = format!(
        "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/purchases/productsv2/tokens/{}",
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
        let product_response = res
            .json::<GooglePlayProductPurchaseV2>()
            .await
            .map_err(|e| AppError::GooglePlayResponseParse(e.to_string()))?;

        Ok(product_response)
    } else {
        Err(AppError::GooglePlayApi(format!(
            "API returned error status: {}",
            res.status()
        )))
    }
}

#[cfg(feature = "local")]
pub async fn consume_google_play_product(
    _package_name: &str,
    _product_id: &str,
    _purchase_token: &str,
    _auth: Option<&Arc<GoogleAuth>>,
) -> AppResult<()> {
    Ok(())
}

#[cfg(not(feature = "local"))]
pub async fn consume_google_play_product(
    package_name: &str,
    product_id: &str,
    purchase_token: &str,
    auth: Option<&Arc<GoogleAuth>>,
) -> AppResult<()> {
    use crate::error::AppError;

    let auth = auth.ok_or(AppError::AuthServiceUnavailable)?;
    let access_token = auth
        .get_token_for_default_scopes()
        .await
        .map_err(|e| AppError::AccessTokenFailed(e.to_string()))?;

    let url = format!(
        "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/purchases/products/{}/tokens/{}:consume",
        package_name, product_id, purchase_token
    );

    let client = reqwest::Client::new();
    let res = client
        .post(&url)
        .bearer_auth(&access_token)
        .header("Content-Type", "application/json")
        .body("{}")
        .send()
        .await
        .map_err(AppError::from)?;

    if res.status().is_success() {
        Ok(())
    } else {
        let error_text = res.text().await.unwrap_or_default();
        Err(AppError::GooglePlayApi(format!(
            "Consume failed: {}",
            error_text
        )))
    }
}
