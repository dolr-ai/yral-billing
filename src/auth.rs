use axum::{
    extract::Request,
    http::{header::AUTHORIZATION, StatusCode},
    middleware::Next,
    response::Response,
};
use google_cloud_auth::credentials::CredentialsFile;
use google_cloud_auth::project::{create_token_source_from_credentials, Config};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::env;

/// Ed25519 public key for JWT verification
pub const JWT_PUBKEY: &str = "-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEAn4Vbu7ZX4fDX3SNCiDYMoOs4KITJP1h2dw+MBnu6pPw=
-----END PUBLIC KEY-----";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub aud: String,
    pub exp: usize,
}

#[derive(Clone)]
pub struct GoogleAuth {
    credentials: CredentialsFile,
}

impl GoogleAuth {
    /// Create a new GoogleAuth instance from environment variables
    /// This is lightweight - just parsing JSON credentials once
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let service_account_json = env::var("GOOGLE_SERVICE_ACCOUNT_JSON")
            .map_err(|_| "GOOGLE_SERVICE_ACCOUNT_JSON environment variable must be set")?;

        let credentials: CredentialsFile = serde_json::from_str(&service_account_json)?;

        Ok(Self { credentials })
    }

    pub async fn get_token(&self, scopes: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
        // Create config with the required scopes
        let config = Config {
            scopes: Some(scopes),
            ..Default::default()
        };

        // Create token source from credentials with scopes
        let token_source = create_token_source_from_credentials(&self.credentials, &config).await?;

        // Get the token
        let token = token_source.token().await?;
        Ok(token.access_token)
    }

    pub async fn get_token_for_default_scopes(&self) -> Result<String, Box<dyn std::error::Error>> {
        // Google Play Android Publisher API scope
        let scopes = &["https://www.googleapis.com/auth/androidpublisher"];
        self.get_token(scopes).await
    }
}

/// JWT authentication middleware
/// Validates JWT token in Authorization header
/// Note: Does not check expiry as per requirements
pub async fn jwt_auth_middleware(req: Request, next: Next) -> Result<Response, StatusCode> {
    // Get Authorization header
    let auth_header = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Extract token from "Bearer <token>"
    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Decode and validate JWT without checking expiry using Ed25519 public key
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.validate_exp = false; // Don't check expiry as per requirements
    validation.validate_aud = false;

    let _claims = decode::<Claims>(
        token,
        &DecodingKey::from_ed_pem(JWT_PUBKEY.as_bytes()).map_err(|_| StatusCode::UNAUTHORIZED)?,
        &validation,
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;

    // Token is valid, continue with request
    Ok(next.run(req).await)
}
