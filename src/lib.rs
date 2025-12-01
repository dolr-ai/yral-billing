pub mod auth;
pub mod model;
pub mod routes;
pub mod schema;
pub mod types;

use auth::GoogleAuth;
use axum::{
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use routes::purchase::verify_purchase;
use routes::rtdn::handle_rtdn_webhook;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub google_auth: Option<Arc<GoogleAuth>>,
}

async fn health_check() -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(serde_json::json!({"status": "ok"})))
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

        let app_state = AppState { google_auth };

        let app = Router::new()
            .route("/health", get(health_check))
            .route("/verify", post(verify_purchase))
            .route("/rtdn-webhook", post(handle_rtdn_webhook))
            .with_state(app_state);

        let port: u16 = env::var("PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse()
            .expect("PORT must be a valid number");

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        println!("Listening on {}", addr);

        axum::serve(
            tokio::net::TcpListener::bind(addr).await.unwrap(),
            app.into_make_service(),
        )
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
