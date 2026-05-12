use std::{env, str::FromStr};

use base64::prelude::*;
#[cfg(not(feature = "local"))]
use chrono::Utc;
#[cfg(not(feature = "local"))]
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
#[cfg(not(feature = "local"))]
use openssl::x509::X509;
use serde::Deserialize;
#[cfg(not(feature = "local"))]
use serde::Serialize;

use crate::{
    error::{AppError, AppResult},
    types::{AppleEnvironment, AppleJWSTransactionDecodedPayload, AppleNotificationDecodedPayload},
};

#[cfg(not(feature = "local"))]
#[derive(Clone)]
pub struct AppleAuthConfig {
    pub issuer_id: String,
    pub key_id: String,
    pub bundle_id: String,
    pub private_key: String,
    pub default_environment: AppleEnvironment,
}

#[cfg(not(feature = "local"))]
impl AppleAuthConfig {
    pub fn from_env() -> AppResult<Self> {
        let default_environment = env::var("APPLE_DEFAULT_ENVIRONMENT")
            .ok()
            .map(|v| AppleEnvironment::from_str(&v))
            .transpose()
            .map_err(AppError::AppleConfig)?
            .unwrap_or(AppleEnvironment::Production);

        Ok(Self {
            issuer_id: env::var("APPLE_ISSUER_ID")
                .map_err(|_| AppError::AppleConfig("APPLE_ISSUER_ID is required".to_string()))?,
            key_id: env::var("APPLE_KEY_ID")
                .map_err(|_| AppError::AppleConfig("APPLE_KEY_ID is required".to_string()))?,
            bundle_id: env::var("APPLE_BUNDLE_ID")
                .map_err(|_| AppError::AppleConfig("APPLE_BUNDLE_ID is required".to_string()))?,
            private_key: env::var("APPLE_PRIVATE_KEY")
                .map_err(|_| AppError::AppleConfig("APPLE_PRIVATE_KEY is required".to_string()))?,
            default_environment,
        })
    }
}

#[cfg(not(feature = "local"))]
#[derive(Debug, Serialize, Deserialize)]
struct AppleApiClaims {
    iss: String,
    iat: i64,
    exp: i64,
    aud: String,
    bid: String,
}

#[cfg(not(feature = "local"))]
fn apple_bearer_token(config: &AppleAuthConfig) -> AppResult<String> {
    let now = Utc::now().timestamp();
    let claims = AppleApiClaims {
        iss: config.issuer_id.clone(),
        iat: now,
        exp: now + 20 * 60,
        aud: "appstoreconnect-v1".to_string(),
        bid: config.bundle_id.clone(),
    };

    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(config.key_id.clone());
    header.typ = Some("JWT".to_string());

    encode(
        &header,
        &claims,
        &EncodingKey::from_ec_pem(config.private_key.as_bytes())
            .map_err(|e| AppError::AppleConfig(format!("Invalid Apple private key: {e}")))?,
    )
    .map_err(|e| AppError::AppleConfig(format!("Failed to sign Apple JWT: {e}")))
}

#[cfg(feature = "local")]
fn decode_jws_payload<T: for<'de> Deserialize<'de>>(jws: &str) -> AppResult<T> {
    let payload = jws
        .split('.')
        .nth(1)
        .ok_or_else(|| AppError::AppleVerification("Invalid JWS format".to_string()))?;
    let bytes = BASE64_URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|e| AppError::AppleVerification(format!("Invalid JWS payload: {e}")))?;

    serde_json::from_slice(&bytes)
        .map_err(|e| AppError::AppleVerification(format!("Failed to parse JWS payload: {e}")))
}

#[cfg(not(feature = "local"))]
#[derive(Debug, Deserialize)]
struct AppleJwsHeader {
    x5c: Option<Vec<String>>,
}

#[cfg(not(feature = "local"))]
fn decode_verified_jws_payload<T>(jws: &str) -> AppResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let header = jws
        .split('.')
        .next()
        .ok_or_else(|| AppError::AppleVerification("Invalid JWS format".to_string()))?;
    let header_bytes = BASE64_URL_SAFE_NO_PAD
        .decode(header)
        .map_err(|e| AppError::AppleVerification(format!("Invalid JWS header: {e}")))?;
    let header = serde_json::from_slice::<AppleJwsHeader>(&header_bytes)
        .map_err(|e| AppError::AppleVerification(format!("Failed to parse JWS header: {e}")))?;
    let leaf_cert = header
        .x5c
        .and_then(|certs| certs.into_iter().next())
        .ok_or_else(|| {
            AppError::AppleVerification("Apple JWS missing x5c certificate".to_string())
        })?;
    let leaf_cert_der = BASE64_STANDARD
        .decode(leaf_cert)
        .map_err(|e| AppError::AppleVerification(format!("Invalid Apple certificate: {e}")))?;
    let cert = X509::from_der(&leaf_cert_der)
        .map_err(|e| AppError::AppleVerification(format!("Invalid Apple certificate DER: {e}")))?;
    let public_key = cert
        .public_key()
        .and_then(|key| key.public_key_to_pem())
        .map_err(|e| AppError::AppleVerification(format!("Invalid Apple public key: {e}")))?;

    let mut validation = Validation::new(Algorithm::ES256);
    validation.validate_exp = false;
    validation.validate_aud = false;
    validation.required_spec_claims.clear();

    decode::<T>(
        jws,
        &DecodingKey::from_ec_pem(&public_key)
            .map_err(|e| AppError::AppleVerification(format!("Invalid Apple EC key: {e}")))?,
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|e| AppError::AppleVerification(format!("Invalid Apple JWS signature: {e}")))
}

pub fn decode_apple_transaction_jws(
    signed_transaction_info: &str,
) -> AppResult<AppleJWSTransactionDecodedPayload> {
    #[cfg(feature = "local")]
    {
        decode_jws_payload(signed_transaction_info)
    }

    #[cfg(not(feature = "local"))]
    {
        decode_verified_jws_payload(signed_transaction_info)
    }
}

pub fn decode_apple_notification_jws(
    signed_payload: &str,
) -> AppResult<AppleNotificationDecodedPayload> {
    #[cfg(feature = "local")]
    {
        decode_jws_payload(signed_payload)
    }

    #[cfg(not(feature = "local"))]
    {
        decode_verified_jws_payload(signed_payload)
    }
}

pub fn validate_apple_transaction(
    transaction: AppleJWSTransactionDecodedPayload,
    expected_transaction_id: &str,
    expected_product_id: &str,
    expected_bundle_id: &str,
) -> AppResult<AppleJWSTransactionDecodedPayload> {
    if transaction.transaction_id != expected_transaction_id {
        return Err(AppError::AppleVerification(format!(
            "Transaction id mismatch: expected {}, got {}",
            expected_transaction_id, transaction.transaction_id
        )));
    }

    if transaction.product_id != expected_product_id {
        return Err(AppError::AppleVerification(format!(
            "Product id mismatch: expected {}, got {}",
            expected_product_id, transaction.product_id
        )));
    }

    if transaction.bundle_id != expected_bundle_id {
        return Err(AppError::AppleVerification(format!(
            "Bundle id mismatch: expected {}, got {}",
            expected_bundle_id, transaction.bundle_id
        )));
    }

    if transaction.revocation_date.is_some() {
        return Err(AppError::AppleVerification(
            "Transaction has been revoked".to_string(),
        ));
    }

    if transaction.app_account_token.is_none() {
        return Err(AppError::ExternalAccountIdentifiersMissing);
    }

    Ok(transaction)
}

#[cfg(feature = "local")]
pub async fn fetch_apple_transaction_info(
    transaction_id: &str,
    product_id: &str,
    _environment: AppleEnvironment,
) -> AppResult<AppleJWSTransactionDecodedPayload> {
    let app_account_token = uuid::Uuid::parse_str(transaction_id)
        .map(|uuid| uuid.to_string())
        .unwrap_or_else(|_| "00000000-0000-0000-0000-000000000001".to_string());

    Ok(AppleJWSTransactionDecodedPayload {
        transaction_id: transaction_id.to_string(),
        original_transaction_id: Some(transaction_id.to_string()),
        bundle_id: "com.example".to_string(),
        product_id: product_id.to_string(),
        app_account_token: Some(app_account_token),
        revocation_date: None,
        expires_date: None,
        environment: Some("Sandbox".to_string()),
    })
}

#[cfg(not(feature = "local"))]
pub async fn fetch_apple_transaction_info(
    transaction_id: &str,
    _product_id: &str,
    environment: AppleEnvironment,
) -> AppResult<AppleJWSTransactionDecodedPayload> {
    let config = AppleAuthConfig::from_env()?;
    let token = apple_bearer_token(&config)?;
    let url = format!(
        "{}/inApps/v1/transactions/{}",
        environment.base_url(),
        transaction_id
    );

    let res = reqwest::Client::new()
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(AppError::from)?;

    if !res.status().is_success() {
        return Err(AppError::AppleApi(format!(
            "Get Transaction Info returned status {}",
            res.status()
        )));
    }

    let response = res
        .json::<crate::types::AppleTransactionInfoResponse>()
        .await
        .map_err(|e| AppError::AppleApi(format!("Failed to parse transaction response: {e}")))?;

    let transaction = decode_apple_transaction_jws(&response.signed_transaction_info)?;
    validate_apple_transaction(transaction, transaction_id, _product_id, &config.bundle_id)
}

#[cfg(feature = "local")]
pub fn default_apple_environment() -> AppleEnvironment {
    env::var("APPLE_DEFAULT_ENVIRONMENT")
        .ok()
        .and_then(|v| AppleEnvironment::from_str(&v).ok())
        .unwrap_or(AppleEnvironment::Sandbox)
}

#[cfg(not(feature = "local"))]
pub fn default_apple_environment() -> AppleEnvironment {
    env::var("APPLE_DEFAULT_ENVIRONMENT")
        .ok()
        .and_then(|v| AppleEnvironment::from_str(&v).ok())
        .unwrap_or(AppleEnvironment::Production)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_environment_base_urls_are_stable() {
        assert_eq!(
            AppleEnvironment::Production.base_url(),
            "https://api.storekit.apple.com"
        );
        assert_eq!(
            AppleEnvironment::Sandbox.base_url(),
            "https://api.storekit-sandbox.apple.com"
        );
    }
}
