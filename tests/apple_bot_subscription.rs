use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::prelude::*;
use diesel::prelude::*;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use tower::ServiceExt;
use uuid;
use yral_billing::model::AppleAppAccountToken;
use yral_billing::routes::apple_chat_access::handle_apple_server_notification;
use yral_billing::routes::bot_subscription::{
    check_bot_subscription, grant_apple_bot_subscription,
};
use yral_billing::routes::transactions::get_balance;
use yral_billing::types::{
    AppleJWSTransactionDecodedPayload, AppleNotificationData, AppleNotificationDecodedPayload,
    AppleServerNotificationRequest, GrantAppleBotSubscriptionRequest,
};
use yral_billing::AppState;

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

// Non-UUID transaction ids make the local Apple mock fall back to this appAccountToken
const DEFAULT_APP_ACCOUNT_TOKEN: &str = "00000000-0000-0000-0000-000000000001";

async fn create_test_app() -> Router {
    let app_state = AppState::new().await;
    Router::new()
        .route(
            "/apple/bot-subscription/grant",
            axum::routing::post(grant_apple_bot_subscription),
        )
        .route(
            "/bot-subscription/check",
            axum::routing::get(check_bot_subscription),
        )
        .route(
            "/apple/server-notifications",
            axum::routing::post(handle_apple_server_notification),
        )
        .route("/transactions/balance", axum::routing::get(get_balance))
        .with_state(app_state)
}

struct TestDbGuard {
    db_path: String,
    original_database_url: Option<String>,
}

impl TestDbGuard {
    fn new() -> Self {
        let test_db = format!("./test_apple_bot_sub_{}.db", uuid::Uuid::new_v4());
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

fn insert_default_apple_account_mapping(db_path: &str, user_id: &str) {
    let mut conn = SqliteConnection::establish(db_path).unwrap();
    let now = chrono::Utc::now().naive_utc();
    let mapping = AppleAppAccountToken {
        id: uuid::Uuid::new_v4().to_string(),
        app_account_token: DEFAULT_APP_ACCOUNT_TOKEN.to_string(),
        user_id: user_id.to_string(),
        created_at: now,
        updated_at: now,
    };

    diesel::insert_into(yral_billing::schema::apple_app_account_tokens::table)
        .values(&mapping)
        .execute(&mut conn)
        .unwrap();
}

fn grant_request(transaction_id: &str, bot_id: &str) -> GrantAppleBotSubscriptionRequest {
    GrantAppleBotSubscriptionRequest {
        transaction_id: transaction_id.to_string(),
        product_id: "mock-product-id".to_string(),
        bot_id: bot_id.to_string(),
        environment: None,
    }
}

async fn post_grant(
    app: Router,
    payload: &GrantAppleBotSubscriptionRequest,
) -> axum::response::Response {
    let req = Request::builder()
        .method("POST")
        .uri("/apple/bot-subscription/grant")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(payload).unwrap()))
        .unwrap();
    app.oneshot(req).await.unwrap()
}

async fn get_json(app: Router, uri: &str) -> serde_json::Value {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body_bytes).unwrap()
}

fn fake_jws<T: serde::Serialize>(payload: &T) -> String {
    let header = BASE64_URL_SAFE_NO_PAD.encode(r#"{"alg":"ES256"}"#);
    let payload = BASE64_URL_SAFE_NO_PAD.encode(serde_json::to_vec(payload).unwrap());
    format!("{header}.{payload}.signature")
}

fn subscription_transaction(
    transaction_id: &str,
    original_transaction_id: &str,
    expires_in_days: i64,
) -> AppleJWSTransactionDecodedPayload {
    AppleJWSTransactionDecodedPayload {
        transaction_id: transaction_id.to_string(),
        original_transaction_id: Some(original_transaction_id.to_string()),
        bundle_id: "com.example".to_string(),
        product_id: "mock-product-id".to_string(),
        app_account_token: Some(DEFAULT_APP_ACCOUNT_TOKEN.to_string()),
        revocation_date: None,
        expires_date: Some(
            (chrono::Utc::now() + chrono::Duration::days(expires_in_days)).timestamp_millis(),
        ),
        environment: Some("Sandbox".to_string()),
    }
}

async fn post_notification(
    app: Router,
    notification_type: &str,
    transaction: &AppleJWSTransactionDecodedPayload,
) -> axum::response::Response {
    let notification_jws = fake_jws(&AppleNotificationDecodedPayload {
        notification_type: notification_type.to_string(),
        subtype: None,
        data: Some(AppleNotificationData {
            signed_transaction_info: Some(fake_jws(transaction)),
        }),
    });

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
    app.oneshot(req).await.unwrap()
}

// Grant succeeds; check reports subscribed with the store-provided expiry
#[tokio::test]
async fn test_grant_apple_bot_subscription_success() {
    let db_guard = TestDbGuard::new();
    insert_default_apple_account_mapping(db_guard.db_path(), "mock-user-id");
    let transaction_id = format!("ios_sub_txn_{}", uuid::Uuid::new_v4());

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&transaction_id, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let response = get_json(
        app,
        "/bot-subscription/check?user_id=mock-user-id&bot_id=bot_abc",
    )
    .await;
    assert_eq!(response["data"]["subscribed"], true);
    assert!(response["data"]["expires_at"].is_string());
}

// Retried grant is idempotent and pays exactly one initial reward
#[tokio::test]
async fn test_grant_apple_bot_subscription_idempotent_single_reward() {
    let db_guard = TestDbGuard::new();
    insert_default_apple_account_mapping(db_guard.db_path(), "mock-user-id");
    let transaction_id = format!("ios_sub_txn_{}", uuid::Uuid::new_v4());
    let payload = grant_request(&transaction_id, "bot_abc");

    let app = create_test_app().await;
    let res = post_grant(app, &payload).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let res = post_grant(app, &payload).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let response = get_json(app, "/transactions/balance?recipient_id=bot_abc").await;
    assert_eq!(response["data"]["balance_paise"], 900);
}

// Same subscription used for a different bot is rejected
#[tokio::test]
async fn test_grant_apple_bot_subscription_different_bot_rejected() {
    let db_guard = TestDbGuard::new();
    insert_default_apple_account_mapping(db_guard.db_path(), "mock-user-id");
    let transaction_id = format!("ios_sub_txn_{}", uuid::Uuid::new_v4());

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&transaction_id, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&transaction_id, "bot_xyz")).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

// DID_RENEW advances expiry and pays one renewal reward; a replay pays nothing
#[tokio::test]
async fn test_apple_did_renew_pays_renewal_reward_once() {
    let db_guard = TestDbGuard::new();
    insert_default_apple_account_mapping(db_guard.db_path(), "mock-user-id");
    let original_txn = format!("ios_sub_txn_{}", uuid::Uuid::new_v4());

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&original_txn, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);

    // Renewal: fresh transaction id, same original id, expiry further out
    let renewal_txn = format!("ios_sub_txn_{}", uuid::Uuid::new_v4());
    let renewal = subscription_transaction(&renewal_txn, &original_txn, 14);

    let app = create_test_app().await;
    let res = post_notification(app, "DID_RENEW", &renewal).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let response = get_json(app, "/transactions/balance?recipient_id=bot_abc").await;
    assert_eq!(response["data"]["balance_paise"], 900 + 6900);

    // Replay of the same renewal payload must not double-pay
    let app = create_test_app().await;
    let res = post_notification(app, "DID_RENEW", &renewal).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let response = get_json(app, "/transactions/balance?recipient_id=bot_abc").await;
    assert_eq!(response["data"]["balance_paise"], 900 + 6900);

    let app = create_test_app().await;
    let response = get_json(
        app,
        "/bot-subscription/check?user_id=mock-user-id&bot_id=bot_abc",
    )
    .await;
    assert_eq!(response["data"]["subscribed"], true);
}

// EXPIRED notification ends access
#[tokio::test]
async fn test_apple_expired_notification_ends_access() {
    let db_guard = TestDbGuard::new();
    insert_default_apple_account_mapping(db_guard.db_path(), "mock-user-id");
    let original_txn = format!("ios_sub_txn_{}", uuid::Uuid::new_v4());

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&original_txn, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);

    let expired = subscription_transaction(&original_txn, &original_txn, 7);
    let app = create_test_app().await;
    let res = post_notification(app, "EXPIRED", &expired).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let response = get_json(
        app,
        "/bot-subscription/check?user_id=mock-user-id&bot_id=bot_abc",
    )
    .await;
    assert_eq!(response["data"]["subscribed"], false);
    assert_eq!(response["data"]["status"], "Expired");
}

// REFUND cancels the subscription; re-granting the same transaction is rejected
#[tokio::test]
async fn test_apple_refund_cancels_subscription() {
    let db_guard = TestDbGuard::new();
    insert_default_apple_account_mapping(db_guard.db_path(), "mock-user-id");
    let original_txn = format!("ios_sub_txn_{}", uuid::Uuid::new_v4());

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&original_txn, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);

    let refunded = subscription_transaction(&original_txn, &original_txn, 7);
    let app = create_test_app().await;
    let res = post_notification(app, "REFUND", &refunded).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let response = get_json(
        app,
        "/bot-subscription/check?user_id=mock-user-id&bot_id=bot_abc",
    )
    .await;
    assert_eq!(response["data"]["subscribed"], false);
    assert_eq!(response["data"]["status"], "Canceled");

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&original_txn, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
