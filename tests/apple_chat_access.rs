use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::prelude::*;
use diesel::prelude::*;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use tower::ServiceExt;
use uuid;
use yral_billing::model::AppleAppAccountToken;
use yral_billing::routes::apple_chat_access::{
    get_or_create_apple_app_account_token, grant_apple_chat_access,
    handle_apple_server_notification,
};
use yral_billing::routes::chat_access::check_chat_access;
use yral_billing::routes::transactions::{get_balance, get_user_transactions};
use yral_billing::types::{
    AppleJWSTransactionDecodedPayload, AppleNotificationData, AppleNotificationDecodedPayload,
    AppleServerNotificationRequest, GrantAppleChatAccessRequest,
};
use yral_billing::AppState;

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

async fn create_test_app() -> Router {
    let app_state = AppState::new().await;
    Router::new()
        .route(
            "/apple/app-account-token",
            axum::routing::post(get_or_create_apple_app_account_token),
        )
        .route(
            "/apple/chat-access/grant",
            axum::routing::post(grant_apple_chat_access),
        )
        .route(
            "/apple/server-notifications",
            axum::routing::post(handle_apple_server_notification),
        )
        .route(
            "/google/chat-access/check",
            axum::routing::get(check_chat_access),
        )
        .route("/transactions", axum::routing::get(get_user_transactions))
        .route("/transactions/balance", axum::routing::get(get_balance))
        .with_state(app_state)
}

struct TestDbGuard {
    db_path: String,
    original_database_url: Option<String>,
}

impl TestDbGuard {
    fn new() -> Self {
        let test_db = format!("./test_apple_chat_{}.db", uuid::Uuid::new_v4());
        let original_database_url = std::env::var("DATABASE_URL").ok();
        unsafe {
            std::env::set_var("DATABASE_URL", &test_db);
        }
        let mut conn = SqliteConnection::establish(&test_db).unwrap();
        conn.run_pending_migrations(MIGRATIONS).unwrap();
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

fn grant_request(transaction_id: &str, bot_id: &str) -> GrantAppleChatAccessRequest {
    GrantAppleChatAccessRequest {
        transaction_id: transaction_id.to_string(),
        product_id: "ios-chat-product".to_string(),
        bot_id: bot_id.to_string(),
        environment: None,
    }
}

fn insert_default_apple_account_mapping(db_path: &str, user_id: &str) {
    let mut conn = SqliteConnection::establish(db_path).unwrap();
    let now = chrono::Utc::now().naive_utc();
    let mapping = AppleAppAccountToken {
        id: uuid::Uuid::new_v4().to_string(),
        app_account_token: "00000000-0000-0000-0000-000000000001".to_string(),
        user_id: user_id.to_string(),
        created_at: now,
        updated_at: now,
    };

    diesel::insert_into(yral_billing::schema::apple_app_account_tokens::table)
        .values(&mapping)
        .execute(&mut conn)
        .unwrap();
}

async fn post_grant(
    app: Router,
    payload: &GrantAppleChatAccessRequest,
) -> axum::response::Response {
    let req = Request::builder()
        .method("POST")
        .uri("/apple/chat-access/grant")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(payload).unwrap()))
        .unwrap();
    app.oneshot(req).await.unwrap()
}

fn fake_jws<T: serde::Serialize>(payload: &T) -> String {
    let header = BASE64_URL_SAFE_NO_PAD.encode(r#"{"alg":"ES256"}"#);
    let payload = BASE64_URL_SAFE_NO_PAD.encode(serde_json::to_vec(payload).unwrap());
    format!("{header}.{payload}.signature")
}

#[tokio::test]
async fn test_grant_apple_chat_access_success() {
    let db_guard = TestDbGuard::new();
    insert_default_apple_account_mapping(db_guard.db_path(), "mock-user-id");
    let app = create_test_app().await;
    let transaction_id = format!("ios_txn_{}", uuid::Uuid::new_v4());

    let res = post_grant(app, &grant_request(&transaction_id, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_grant_apple_chat_access_idempotent() {
    let db_guard = TestDbGuard::new();
    insert_default_apple_account_mapping(db_guard.db_path(), "mock-user-id");
    let transaction_id = format!("ios_txn_{}", uuid::Uuid::new_v4());
    let payload = grant_request(&transaction_id, "bot_abc");

    let app = create_test_app().await;
    let res = post_grant(app, &payload).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let res = post_grant(app, &payload).await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_grant_apple_chat_access_different_bot_rejected() {
    let db_guard = TestDbGuard::new();
    insert_default_apple_account_mapping(db_guard.db_path(), "mock-user-id");
    let transaction_id = format!("ios_txn_{}", uuid::Uuid::new_v4());

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&transaction_id, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&transaction_id, "bot_xyz")).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_apple_chat_access_check_and_balance() {
    let db_guard = TestDbGuard::new();
    insert_default_apple_account_mapping(db_guard.db_path(), "mock-user-id");
    let transaction_id = format!("ios_txn_{}", uuid::Uuid::new_v4());

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&transaction_id, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let req = Request::builder()
        .method("GET")
        .uri("/google/chat-access/check?user_id=mock-user-id&bot_id=bot_abc")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let response: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(response["data"]["has_access"], true);

    let app = create_test_app().await;
    let req = Request::builder()
        .method("GET")
        .uri("/transactions/balance?recipient_id=bot_abc")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let response: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(response["data"]["balance_paise"], 900);
}

#[tokio::test]
async fn test_apple_refund_notification_cancels_access() {
    let db_guard = TestDbGuard::new();
    insert_default_apple_account_mapping(db_guard.db_path(), "mock-user-id");
    let transaction_id = format!("ios_txn_{}", uuid::Uuid::new_v4());

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&transaction_id, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);

    let transaction_jws = fake_jws(&AppleJWSTransactionDecodedPayload {
        transaction_id: transaction_id.clone(),
        original_transaction_id: Some(transaction_id.clone()),
        bundle_id: "com.example".to_string(),
        product_id: "ios-chat-product".to_string(),
        app_account_token: Some("00000000-0000-0000-0000-000000000001".to_string()),
        revocation_date: Some(1),
        expires_date: None,
        environment: Some("Sandbox".to_string()),
    });
    let notification_jws = fake_jws(&AppleNotificationDecodedPayload {
        notification_type: "REFUND".to_string(),
        subtype: None,
        data: Some(AppleNotificationData {
            signed_transaction_info: Some(transaction_jws),
        }),
    });

    let app = create_test_app().await;
    let req = Request::builder()
        .method("POST")
        .uri("/apple/server-notifications")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&AppleServerNotificationRequest {
                signed_payload: notification_jws,
            })
            .unwrap(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let req = Request::builder()
        .method("GET")
        .uri("/google/chat-access/check?user_id=mock-user-id&bot_id=bot_abc")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let response: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(response["data"]["has_access"], false);
}

#[tokio::test]
async fn test_apple_app_account_token_endpoint_is_stable() {
    let _db_guard = TestDbGuard::new();
    let app = create_test_app().await;

    let payload = serde_json::json!({ "user_id": "user-token-test" });
    let req = Request::builder()
        .method("POST")
        .uri("/apple/app-account-token")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let response: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let first = response["data"]["app_account_token"].as_str().unwrap();
    assert!(uuid::Uuid::parse_str(first).is_ok());

    let app = create_test_app().await;
    let req = Request::builder()
        .method("POST")
        .uri("/apple/app-account-token")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let response: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(response["data"]["app_account_token"], first);
}

#[tokio::test]
async fn test_grant_apple_chat_access_unknown_app_account_token_rejected() {
    let _db_guard = TestDbGuard::new();
    let app = create_test_app().await;
    let transaction_id = format!("ios_txn_{}", uuid::Uuid::new_v4());

    let res = post_grant(app, &grant_request(&transaction_id, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
