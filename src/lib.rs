pub mod auth;
pub mod error;
pub mod model;
pub mod routes;
pub mod schema;
pub mod types;

use auth::{GoogleAuth, jwt_auth_middleware};
use axum::{
    http::StatusCode,
    middleware,
    response::{Html, IntoResponse, Json, Redirect},
    routing::{get, post},
    Router,
};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use routes::purchase::verify_purchase;
use routes::rtdn::handle_rtdn_webhook;
use routes::credits::{deduct_credits, increment_credits};
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use types::{AckData, AckRequest, ApiResponse, CreditRequest, EmptyData, PurchaseTokenStatus, VerifyRequest};
use utoipa::OpenApi;

use crate::types::VerifyResponse;

#[derive(Clone)]
pub struct AppState {
    pub google_auth: Option<Arc<GoogleAuth>>,
    pub admin_ic_agent: Option<ic_agent::Agent>,
}

impl AppState {
    /// Get a database connection
    pub fn get_db_connection(&self) -> Result<SqliteConnection, diesel::ConnectionError> {
        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| "billing.db".to_string());
        SqliteConnection::establish(&database_url)
    }
}

#[derive(OpenApi)]
#[openapi(
    paths(
        routes::purchase::verify_purchase,
        routes::credits::deduct_credits,
        routes::credits::increment_credits,
        health_check
    ),
    components(
        schemas(ApiResponse<EmptyData>, EmptyData, VerifyRequest, VerifyResponse, AckRequest, AckData, PurchaseTokenStatus, CreditRequest)
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "Subscription Verification", description = "Google Play subscription verification endpoints"),
        (name = "Credits", description = "User credit management endpoints"),
        (name = "Health", description = "Health check endpoints")
    ),
    info(
        title = "YRAL Billing API",
        version = "1.0.0",
        description = "API for handling Google Play subscription billing operations and user credit management",
        contact(
            name = "YRAL Team",
            url = "https://yral.com"
        )
    )
)]
struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                utoipa::openapi::security::SecurityScheme::Http(
                    utoipa::openapi::security::Http::new(
                        utoipa::openapi::security::HttpAuthScheme::Bearer,
                    ),
                ),
            )
        }
    }
}

#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "Service is healthy", body = serde_json::Value)
    ),
    tag = "Health"
)]
async fn health_check() -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(serde_json::json!({"status": "ok"})))
}

async fn openapi_spec() -> impl IntoResponse {
    Json(ApiDoc::openapi())
}

async fn swagger_ui() -> impl IntoResponse {
    Html(include_str!("../static/swagger.html"))
}

async fn root_redirect() -> Redirect {
    Redirect::permanent("/explore")
}

pub fn run() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        // Run database migrations on startup
        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| "billing.db".to_string());
        if let Err(e) = run_migrations(&database_url) {
            eprintln!("Failed to run migrations: {}", e);
            std::process::exit(1);
        }

        // Initialize Google Auth (only for production, not for local/mock features)
        let google_auth = if cfg!(any(feature = "local", feature = "mock-google-api")) {
            None
        } else {
            match GoogleAuth::from_env() {
                Ok(auth) => {
                    println!("Google Auth initialized successfully");
                    Some(Arc::new(auth))
                }
                Err(e) => {
                    eprintln!("Failed to initialize Google Auth: {}", e);
                    std::process::exit(1);
                }
            }
        };

        let admin_ic_agent = if cfg!(any(feature = "local", feature = "mock-google-api")) {
            None
        } else {
            let backend_admin_secret_key = env::var("BACKEND_ADMIN_SECRET_KEY")
                .expect("expect backend admin canister key to be present");

            let identity = match ic_agent::identity::Secp256k1Identity::from_pem(
                stringreader::StringReader::new(backend_admin_secret_key.as_str()),
            ) {
                Ok(identity) => identity,
                Err(err) => {
                    panic!("Unable to create identity, error: {err:?}");
                }
            };

            let admin_ic_agent = ic_agent::Agent::builder()
                .with_url("https://ic0.app")
                .with_identity(identity)
                .build()
                .expect("Failed to create IC agent for admin canister");
            Some(admin_ic_agent)
        };

        let app_state = AppState {
            google_auth,
            admin_ic_agent,
        };
        // Create protected routes with JWT middleware
        let protected_routes = Router::new()
            .route("/credits/deduct", post(deduct_credits))
            .route("/credits/increment", post(increment_credits))
            .layer(middleware::from_fn(jwt_auth_middleware));

        let app = Router::new()
            .route("/", get(root_redirect))
            .route("/health", get(health_check))
            .route("/google/verify", post(verify_purchase))
            .route("/google/rtdn-webhook", post(handle_rtdn_webhook))
            .route("/api-doc/openapi.json", get(openapi_spec))
            .route("/explore", get(swagger_ui))
            .merge(protected_routes)
            .with_state(app_state);

        let port: u16 = env::var("PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse()
            .expect("PORT must be a valid number");

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        println!("Listening on {}", addr);

        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    });
}

fn run_migrations(database_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

    const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

    let mut connection = SqliteConnection::establish(database_url)?;
    connection
        .run_pending_migrations(MIGRATIONS)
        .map_err(|e| format!("Migration error: {}", e))?;

    println!("Database migrations completed successfully");
    Ok(())
}
