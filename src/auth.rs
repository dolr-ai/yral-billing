use google_cloud_auth::credentials::CredentialsFile;
use google_cloud_auth::project::{create_token_source_from_credentials, Config};
use std::env;

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
