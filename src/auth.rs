use axum::{
    extract::Request,
    http::{header::AUTHORIZATION, StatusCode},
    middleware::Next,
    response::Response,
};
use chrono::{Date, DateTime};
use diesel::dsl::max;
use google_cloud_auth::credentials::CredentialsFile;
use google_cloud_auth::project::{create_token_source_from_credentials, Config};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use reqwest::header::{CACHE_CONTROL, EXPIRES};
use serde::{Deserialize, Serialize};
use std::{env, f32::consts::E};
use tokio::sync::RwLock;

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

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleClaims {
    iss: String,
    aud: String,
    email: String,
    sub: String,
    exp: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GoogleJwk {
    kty: String,
    kid: String,
    alg: String,
    n: String,
    e: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JwkResponse {
    keys: Vec<GoogleJwk>,
    #[serde(skip, default)]
    expiry: DateTime<chrono::Utc>,
}

#[derive(Debug)]
pub struct GooglePublicKey {
    keys: RwLock<JwkResponse>,
}

impl GooglePublicKey {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let google_public_key = Self {
            keys: RwLock::new(JwkResponse {
                keys: vec![],
                expiry: chrono::Utc::now(),
            }),
        };
        google_public_key.fetch_google_public_keys().await?;
        Ok(google_public_key)
    }

    async fn fetch_google_public_keys(&self) -> Result<(), Box<dyn std::error::Error>> {
        let response = reqwest::get("https://www.googleapis.com/oauth2/v3/certs").await?;
        let headers = response.headers();
        let expiry = headers
            .get(EXPIRES)
            .map(|expires| {
                let expires_str = expires.to_str().unwrap_or("");
                DateTime::parse_from_rfc2822(expires_str)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now() + chrono::Duration::hours(1))
            })
            .unwrap_or(chrono::Utc::now() + chrono::Duration::hours(1));

        let mut jwks: JwkResponse = response.json().await?;
        jwks.expiry = expiry;
        let mut keys = self.keys.write().await;
        *keys = jwks;

        println!(
            "Fetched Google public keys {:?}, expires at {}",
            keys.keys, expiry
        );

        Ok(())
    }

    pub async fn validate_token(
        &self,
        token: &str,
    ) -> Result<GoogleClaims, Box<dyn std::error::Error>> {
        // Decode the JWT header to get the kid

        if self.keys.read().await.expiry < chrono::Utc::now() {
            // Keys have expired, fetch new ones
            self.fetch_google_public_keys().await?;
        }

        let header = jsonwebtoken::decode_header(token)?;
        let kid = header.kid.ok_or("Token missing kid")?;

        let keys = self.keys.read().await;

        // Find the corresponding JWK
        let jwk = keys
            .keys
            .iter()
            .find(|jwk| jwk.kid == kid)
            .ok_or("No matching JWK found")?;

        // Construct the public key from n and e
        let decoding_key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)?;

        // Validate the token and extract claims
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&["https://accounts.google.com", "account.google.com"]);
        validation.set_audience(&["https://billing.yral.com"]);
        validation.validate_exp = true;

        let token_data = decode::<GoogleClaims>(token, &decoding_key, &validation)?;
        Ok(token_data.claims)
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
