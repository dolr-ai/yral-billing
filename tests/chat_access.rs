use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use diesel::prelude::*;
use tower::ServiceExt; // for `oneshot`
use uuid;
use yral_billing::routes::chat_access::{check_chat_access, grant_chat_access};
use yral_billing::types::{BotChatAccessStatus, GrantChatAccessRequest};
use yral_billing::AppState;

async fn create_test_app() -> Router {
    let app_state = AppState::new().await;
    Router::new()
        .route(
            "/google/chat-access/grant",
            axum::routing::post(grant_chat_access),
        )
        .route(
            "/google/chat-access/check",
            axum::routing::get(check_chat_access),
        )
        .with_state(app_state)
}

struct TestDbGuard {
    db_path: String,
    original_database_url: Option<String>,
}

impl TestDbGuard {
    fn new() -> Self {
        let test_db = format!("./test_chat_{}.db", uuid::Uuid::new_v4());
        let original_database_url = std::env::var("DATABASE_URL").ok();
        unsafe {
            std::env::set_var("DATABASE_URL", &test_db);
        }
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
        let _ = std::fs::remove_file(&self.db_path);
        match &self.original_database_url {
            Some(url) => unsafe { std::env::set_var("DATABASE_URL", url) },
            None => std::env::remove_var("DATABASE_URL"),
        }
    }
}

fn grant_request(purchase_token: &str, bot_id: &str) -> GrantChatAccessRequest {
    GrantChatAccessRequest {
        package_name: "com.example".to_string(),
        product_id: "daily_bot_access".to_string(),
        purchase_token: purchase_token.to_string(),
        bot_id: bot_id.to_string(),
    }
}

async fn post_grant(app: Router, payload: &GrantChatAccessRequest) -> axum::response::Response {
    let req = Request::builder()
        .method("POST")
        .uri("/google/chat-access/grant")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(payload).unwrap()))
        .unwrap();
    app.oneshot(req).await.unwrap()
}

async fn get_check(app: Router, user_id: &str, bot_id: &str) -> axum::response::Response {
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/google/chat-access/check?user_id={}&bot_id={}",
            user_id, bot_id
        ))
        .body(Body::empty())
        .unwrap();
    app.oneshot(req).await.unwrap()
}

// Grant succeeds and returns 200
#[tokio::test]
async fn test_grant_chat_access_success() {
    let _db_guard = TestDbGuard::new();
    let app = create_test_app().await;

    let token = format!("token_{}", uuid::Uuid::new_v4());
    let res = post_grant(app, &grant_request(&token, "bot_abc")).await;

    assert_eq!(res.status(), StatusCode::OK);
}

// Calling grant twice with same token + same bot is idempotent — returns 200
#[tokio::test]
async fn test_grant_chat_access_idempotent() {
    let _db_guard = TestDbGuard::new();
    let token = format!("token_{}", uuid::Uuid::new_v4());
    let payload = grant_request(&token, "bot_abc");

    // First grant
    let app = create_test_app().await;
    let res = post_grant(app, &payload).await;
    assert_eq!(res.status(), StatusCode::OK);

    // Second grant — same token, same bot
    let app = create_test_app().await;
    let res = post_grant(app, &payload).await;
    assert_eq!(res.status(), StatusCode::OK);
}

// Same token used for a different bot returns 400 TokenAlreadyUsed
#[tokio::test]
async fn test_grant_chat_access_different_bot_rejected() {
    let _db_guard = TestDbGuard::new();
    let token = format!("token_{}", uuid::Uuid::new_v4());

    // Grant for bot_abc
    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&token, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);

    // Attempt to use same token for bot_xyz
    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&token, "bot_xyz")).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Purchase token already used by different user"));
}

// Check returns has_access=true after a successful grant
// Note: the mock returns obfuscated_external_account_id = "mock-user-id"
#[tokio::test]
async fn test_check_chat_access_active() {
    let _db_guard = TestDbGuard::new();
    let token = format!("token_{}", uuid::Uuid::new_v4());

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&token, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let res = get_check(app, "mock-user-id", "bot_abc").await;
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let response: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(response["data"]["has_access"], true);
    assert!(response["data"]["expires_at"].is_string());
}

// Check returns has_access=false when no grant exists
#[tokio::test]
async fn test_check_chat_access_no_grant() {
    let _db_guard = TestDbGuard::new();
    let app = create_test_app().await;

    let res = get_check(app, "unknown-user", "bot_abc").await;
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let response: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(response["data"]["has_access"], false);
    assert!(response["data"]["expires_at"].is_null());
}

// Check returns has_access=false when grant has been canceled
#[tokio::test]
async fn test_check_chat_access_canceled() {
    use yral_billing::schema::bot_chat_access::dsl;

    let db_guard = TestDbGuard::new();
    let token = format!("token_{}", uuid::Uuid::new_v4());

    // Grant access
    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&token, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);

    // Simulate cancellation by setting status = Canceled directly in DB
    let mut conn = SqliteConnection::establish(db_guard.db_path()).unwrap();
    let now = chrono::Utc::now().naive_utc();
    diesel::update(dsl::bot_chat_access.filter(dsl::purchase_token.eq(&token)))
        .set((
            dsl::status.eq(BotChatAccessStatus::Canceled),
            dsl::updated_at.eq(now),
        ))
        .execute(&mut conn)
        .unwrap();

    // Check should now return false
    let app = create_test_app().await;
    let res = get_check(app, "mock-user-id", "bot_abc").await;
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let response: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(response["data"]["has_access"], false);
}

// Check returns has_access=false when grant has expired
#[tokio::test]
async fn test_check_chat_access_expired() {
    use yral_billing::model::BotChatAccess;
    use yral_billing::schema::bot_chat_access;

    let db_guard = TestDbGuard::new();
    let token = format!("token_{}", uuid::Uuid::new_v4());

    // Insert a grant that already expired
    let mut conn = SqliteConnection::establish(db_guard.db_path()).unwrap();
    let expired_at = (chrono::Utc::now() - chrono::Duration::hours(1)).naive_utc();
    let grant = BotChatAccess::new(token.clone(), "mock-user-id".to_string(), "bot_abc".to_string(), expired_at);
    diesel::insert_into(bot_chat_access::table)
        .values(&grant)
        .execute(&mut conn)
        .unwrap();

    let app = create_test_app().await;
    let res = get_check(app, "mock-user-id", "bot_abc").await;
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let response: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(response["data"]["has_access"], false);
}
