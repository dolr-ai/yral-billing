use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use diesel::prelude::*;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use tower::ServiceExt; // for `oneshot`
use uuid;
use yral_billing::model::BotSubscription;
use yral_billing::routes::bot_subscription::{
    check_bot_subscription, verify_google_bot_subscription,
};
use yral_billing::routes::chat_access::check_chat_access;
use yral_billing::routes::image_access::check_image_access_batch;
use yral_billing::routes::transactions::get_balance;
use yral_billing::types::{BotSubscriptionStatus, PurchaseSource, VerifyBotSubscriptionRequest};
use yral_billing::AppState;

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

// The local subscriptionsv2 mock returns obfuscated_external_account_id = "mock-obfuscated-id"
const MOCK_USER: &str = "mock-obfuscated-id";

async fn create_test_app() -> Router {
    let app_state = AppState::new().await;
    Router::new()
        .route(
            "/google/bot-subscription/verify",
            axum::routing::post(verify_google_bot_subscription),
        )
        .route(
            "/bot-subscription/check",
            axum::routing::get(check_bot_subscription),
        )
        .route(
            "/google/chat-access/check",
            axum::routing::get(check_chat_access),
        )
        .route(
            "/image-access/check-batch",
            axum::routing::post(check_image_access_batch),
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
        let test_db = format!("./test_bot_sub_{}.db", uuid::Uuid::new_v4());
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

fn verify_request(purchase_token: &str, bot_id: &str) -> VerifyBotSubscriptionRequest {
    VerifyBotSubscriptionRequest {
        package_name: "com.example".to_string(),
        product_id: "mock-product-id".to_string(),
        purchase_token: purchase_token.to_string(),
        bot_id: bot_id.to_string(),
    }
}

async fn post_verify(
    app: Router,
    payload: &VerifyBotSubscriptionRequest,
) -> axum::response::Response {
    let req = Request::builder()
        .method("POST")
        .uri("/google/bot-subscription/verify")
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

async fn post_check_batch(app: Router, payload: serde_json::Value) -> serde_json::Value {
    let req = Request::builder()
        .method("POST")
        .uri("/image-access/check-batch")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body_bytes).unwrap()
}

// Verify succeeds and the subscription check reports it active with an expiry
#[tokio::test]
async fn test_verify_bot_subscription_success() {
    let _db_guard = TestDbGuard::new();
    let token = format!("sub_token_{}", uuid::Uuid::new_v4());

    let app = create_test_app().await;
    let res = post_verify(app, &verify_request(&token, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let response = get_json(
        app,
        &format!("/bot-subscription/check?user_id={MOCK_USER}&bot_id=bot_abc"),
    )
    .await;
    assert_eq!(response["data"]["subscribed"], true);
    assert_eq!(response["data"]["status"], "Active");
    assert!(response["data"]["expires_at"].is_string());
}

// Retrying verify with the same token + bot is idempotent and pays exactly one initial reward
#[tokio::test]
async fn test_verify_bot_subscription_idempotent_single_reward() {
    let _db_guard = TestDbGuard::new();
    let token = format!("sub_token_{}", uuid::Uuid::new_v4());
    let payload = verify_request(&token, "bot_abc");

    let app = create_test_app().await;
    let res = post_verify(app, &payload).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let res = post_verify(app, &payload).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let response = get_json(app, "/transactions/balance?recipient_id=bot_abc").await;
    assert_eq!(response["data"]["balance_paise"], 900);
}

// Same subscription token used for a different bot is rejected
#[tokio::test]
async fn test_verify_bot_subscription_different_bot_rejected() {
    let _db_guard = TestDbGuard::new();
    let token = format!("sub_token_{}", uuid::Uuid::new_v4());

    let app = create_test_app().await;
    let res = post_verify(app, &verify_request(&token, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let res = post_verify(app, &verify_request(&token, "bot_xyz")).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

// Product IDs outside the bot-subscription prefix are rejected before any store call
#[tokio::test]
async fn test_verify_bot_subscription_invalid_product_rejected() {
    let _db_guard = TestDbGuard::new();
    let app = create_test_app().await;

    let mut payload = verify_request("some-token", "bot_abc");
    payload.product_id = "other-product".to_string();

    let res = post_verify(app, &payload).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

// An active subscription makes the chat-access check pass with no bot_chat_access row
#[tokio::test]
async fn test_chat_access_check_short_circuits_on_subscription() {
    let _db_guard = TestDbGuard::new();
    let token = format!("sub_token_{}", uuid::Uuid::new_v4());

    let app = create_test_app().await;
    let res = post_verify(app, &verify_request(&token, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let response = get_json(
        app,
        &format!("/google/chat-access/check?user_id={MOCK_USER}&bot_id=bot_abc"),
    )
    .await;
    assert_eq!(response["data"]["has_access"], true);
    assert!(response["data"]["expires_at"].is_string());

    // A different bot is unaffected
    let app = create_test_app().await;
    let response = get_json(
        app,
        &format!("/google/chat-access/check?user_id={MOCK_USER}&bot_id=bot_other"),
    )
    .await;
    assert_eq!(response["data"]["has_access"], false);
}

// With bot_id + active subscription every requested image is unlocked;
// without bot_id (or without a subscription) the per-image fallback applies
#[tokio::test]
async fn test_image_check_batch_short_circuits_on_subscription() {
    let _db_guard = TestDbGuard::new();
    let token = format!("sub_token_{}", uuid::Uuid::new_v4());

    let app = create_test_app().await;
    let res = post_verify(app, &verify_request(&token, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);

    // Subscribed bot: all true
    let app = create_test_app().await;
    let response = post_check_batch(
        app,
        serde_json::json!({
            "user_id": MOCK_USER,
            "image_ids": ["img1", "img2"],
            "bot_id": "bot_abc",
        }),
    )
    .await;
    assert_eq!(response["data"]["access"]["img1"], true);
    assert_eq!(response["data"]["access"]["img2"], true);

    // No bot_id: falls back to per-image rows (none exist)
    let app = create_test_app().await;
    let response = post_check_batch(
        app,
        serde_json::json!({
            "user_id": MOCK_USER,
            "image_ids": ["img1"],
        }),
    )
    .await;
    assert_eq!(response["data"]["access"]["img1"], false);

    // bot_id without a subscription: also falls back
    let app = create_test_app().await;
    let response = post_check_batch(
        app,
        serde_json::json!({
            "user_id": MOCK_USER,
            "image_ids": ["img1"],
            "bot_id": "bot_other",
        }),
    )
    .await;
    assert_eq!(response["data"]["access"]["img1"], false);
}

// Expired/canceled/lapsed rows do not grant access
#[tokio::test]
async fn test_check_bot_subscription_inactive_states() {
    let db_guard = TestDbGuard::new();
    let mut conn = SqliteConnection::establish(db_guard.db_path()).unwrap();

    // Active status but expiry in the past
    let mut lapsed = BotSubscription::new(
        PurchaseSource::Google,
        format!("sub_token_{}", uuid::Uuid::new_v4()),
        MOCK_USER.to_string(),
        "bot_lapsed".to_string(),
        "mock-product-id".to_string(),
        (chrono::Utc::now() - chrono::Duration::hours(1)).naive_utc(),
    );
    lapsed.status = BotSubscriptionStatus::Active;

    // Unexpired but explicitly Expired status
    let mut expired = BotSubscription::new(
        PurchaseSource::Google,
        format!("sub_token_{}", uuid::Uuid::new_v4()),
        MOCK_USER.to_string(),
        "bot_expired".to_string(),
        "mock-product-id".to_string(),
        (chrono::Utc::now() + chrono::Duration::days(7)).naive_utc(),
    );
    expired.status = BotSubscriptionStatus::Expired;

    diesel::insert_into(yral_billing::schema::bot_subscriptions::table)
        .values(&vec![lapsed, expired])
        .execute(&mut conn)
        .unwrap();

    let app = create_test_app().await;
    let response = get_json(
        app,
        &format!("/bot-subscription/check?user_id={MOCK_USER}&bot_id=bot_lapsed"),
    )
    .await;
    assert_eq!(response["data"]["subscribed"], false);
    assert_eq!(response["data"]["status"], "Active");

    let app = create_test_app().await;
    let response = get_json(
        app,
        &format!("/bot-subscription/check?user_id={MOCK_USER}&bot_id=bot_expired"),
    )
    .await;
    assert_eq!(response["data"]["subscribed"], false);
    assert_eq!(response["data"]["status"], "Expired");

    // Chat access check must not short-circuit for either
    let app = create_test_app().await;
    let response = get_json(
        app,
        &format!("/google/chat-access/check?user_id={MOCK_USER}&bot_id=bot_lapsed"),
    )
    .await;
    assert_eq!(response["data"]["has_access"], false);
}
