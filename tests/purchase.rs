use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use tower::ServiceExt; // for `oneshot`
use uuid;
use yral_billing::routes::purchase::verify_purchase;
use yral_billing::types::VerifyRequest;
use yral_billing::AppState;

// Helper function to create a test router with mock state
fn create_test_app() -> Router {
    let app_state = AppState {
        google_auth: None, // Mock state - no auth needed for tests
        admin_ic_agent: None,
    };
    Router::new()
        .route("/verify", axum::routing::post(verify_purchase))
        .with_state(app_state)
}

// Helper struct to ensure test database cleanup
struct TestDbGuard {
    db_path: String,
    original_database_url: Option<String>,
}

impl TestDbGuard {
    fn new() -> Self {
        let test_db = format!("test_{}.db", uuid::Uuid::new_v4());

        // Save original DATABASE_URL if it exists
        let original_database_url = std::env::var("DATABASE_URL").ok();

        // Set test database URL
        unsafe {
            std::env::set_var("DATABASE_URL", &test_db);
        }

        // Run migration on test database
        let _ = std::process::Command::new("diesel")
            .args(&[
                "migration",
                "run",
                "--database-url",
                &format!("sqlite://{}", test_db),
            ])
            .output();

        Self {
            db_path: test_db,
            original_database_url,
        }
    }

    fn db_path(&self) -> &str {
        &self.db_path
    }
}

impl Drop for TestDbGuard {
    fn drop(&mut self) {
        // Clean up test database file
        let _ = std::fs::remove_file(&self.db_path);

        // Restore original DATABASE_URL or remove it
        match &self.original_database_url {
            Some(url) => unsafe {
                std::env::set_var("DATABASE_URL", url);
            },
            None => {
                std::env::remove_var("DATABASE_URL");
            }
        }
    }
}

#[tokio::test]
async fn test_verify_purchase_route() {
    // Set up test database with automatic cleanup
    let _db_guard = TestDbGuard::new();

    let app = create_test_app();

    let payload = VerifyRequest {
        user_id: format!("test_user_{}", uuid::Uuid::new_v4()),
        package_name: "com.example".to_string(),
        product_id: "test_product".to_string(),
        purchase_token: format!("test_token_{}", uuid::Uuid::new_v4()),
    };
    let req = Request::builder()
        .method("POST")
        .uri("/verify")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    // The route should successfully store the new token in the database

    assert_eq!(res.status(), StatusCode::OK);
    // Database cleanup handled automatically by TestDbGuard
}

#[tokio::test]
async fn test_purchase_token_reuse_prevention() {
    use yral_billing::model::PurchaseToken;
    use yral_billing::schema::purchase_tokens;
    use yral_billing::types::PurchaseTokenStatus;

    // Set up test database with automatic cleanup
    let db_guard = TestDbGuard::new();

    let app = create_test_app();

    // Use unique token per test to avoid conflicts
    let shared_token = format!("shared_token_{}", uuid::Uuid::new_v4());

    // Manually insert a token for user_1 to simulate a previous successful verification
    use diesel::prelude::*;
    let mut conn = SqliteConnection::establish(db_guard.db_path()).unwrap();
    let expiry_at = (chrono::Utc::now() + chrono::Duration::days(30)).naive_utc();
    let new_token = PurchaseToken::new(
        "user_1".to_string(),
        shared_token.clone(),
        expiry_at,
        PurchaseTokenStatus::AccessGranted,
    );
    let _ = diesel::insert_into(purchase_tokens::table)
        .values(&new_token)
        .execute(&mut conn);

    // Now test: Second user attempts to use the same purchase token
    let payload_user2 = VerifyRequest {
        user_id: "user_2".to_string(),
        package_name: "com.example".to_string(),
        product_id: "test_product".to_string(),
        purchase_token: shared_token.clone(),
    };

    let req2 = Request::builder()
        .method("POST")
        .uri("/verify")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload_user2).unwrap()))
        .unwrap();

    // Second request should fail (token already used by different user)
    let res2 = app.oneshot(req2).await.unwrap();
    assert_eq!(res2.status(), StatusCode::BAD_REQUEST);

    // Verify the response body contains appropriate error message
    let body_bytes = axum::body::to_bytes(res2.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Purchase token already used by different user"));
    // Database cleanup handled automatically by TestDbGuard
}

#[tokio::test]
async fn test_same_user_same_token_allowed() {
    use yral_billing::model::PurchaseToken;
    use yral_billing::schema::purchase_tokens;
    use yral_billing::types::PurchaseTokenStatus;

    // Set up test database with automatic cleanup
    let db_guard = TestDbGuard::new();

    let app = create_test_app();

    // Use unique token per test to avoid conflicts
    let token = format!("user_token_{}", uuid::Uuid::new_v4());
    let user_id = format!("user_{}", uuid::Uuid::new_v4());

    // Manually insert a token for the user to simulate a previous successful verification
    use diesel::prelude::*;
    let mut conn = SqliteConnection::establish(db_guard.db_path()).unwrap();
    let expiry_at = (chrono::Utc::now() + chrono::Duration::days(30)).naive_utc();
    let new_token = PurchaseToken::new(
        user_id.clone(),
        token.clone(),
        expiry_at,
        PurchaseTokenStatus::AccessGranted,
    );
    let _ = diesel::insert_into(purchase_tokens::table)
        .values(&new_token)
        .execute(&mut conn);

    // Same user uses the same token again
    let payload = VerifyRequest {
        user_id: user_id.clone(),
        package_name: "com.example".to_string(),
        product_id: "test_product".to_string(),
        purchase_token: token.clone(),
    };

    let req = Request::builder()
        .method("POST")
        .uri("/verify")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    // Request from same user should succeed (idempotent behavior)
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Verify the response body indicates it was already verified
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert_eq!(
        body_str.contains("Purchase already verified and access granted"),
        true
    );
    // Database cleanup handled automatically by TestDbGuard
}
