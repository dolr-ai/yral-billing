use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use diesel::prelude::*;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use tower::ServiceExt;
use uuid;
use yral_billing::consts::BOT_SUBSCRIPTION_REWARD_PAISE;
use yral_billing::routes::chat_access::grant_chat_access;
use yral_billing::routes::transactions::{get_balance, get_user_transactions};
use yral_billing::types::GrantChatAccessRequest;
use yral_billing::AppState;

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

async fn create_test_app() -> Router {
    let app_state = AppState::new().await;
    Router::new()
        .route(
            "/google/chat-access/grant",
            axum::routing::post(grant_chat_access),
        )
        .route("/transactions", axum::routing::get(get_user_transactions))
        .route(
            "/transactions/balance",
            axum::routing::get(get_balance),
        )
        .with_state(app_state)
}

struct TestDbGuard {
    db_path: String,
    original_database_url: Option<String>,
}

impl TestDbGuard {
    fn new() -> Self {
        let test_db = format!("./test_txn_{}.db", uuid::Uuid::new_v4());
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

fn grant_request(purchase_token: &str, bot_id: &str) -> GrantChatAccessRequest {
    GrantChatAccessRequest {
        package_name: "com.example".to_string(),
        product_id: "mock-product-id".to_string(),
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

// After a successful grant, a transaction row exists in the DB with the correct fields
#[tokio::test]
async fn test_reward_row_created_on_grant() {
    use yral_billing::schema::transactions::dsl;

    let db_guard = TestDbGuard::new();
    let token = format!("token_{}", uuid::Uuid::new_v4());
    let bot_id = "bot_reward_test";

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&token, bot_id)).await;
    assert_eq!(res.status(), StatusCode::OK);

    let mut conn = SqliteConnection::establish(db_guard.db_path()).unwrap();
    let rows: Vec<(String, i64, String, String)> = dsl::transactions
        .select((
            dsl::user_id,
            dsl::amount_paise,
            dsl::recipient_id,
            dsl::purchase_token,
        ))
        .load(&mut conn)
        .unwrap();

    assert_eq!(rows.len(), 1);
    let (user_id, amount, recipient_id, pt) = &rows[0];
    assert_eq!(user_id, "mock-user-id"); // set by the mock Google Play response
    assert_eq!(*amount, BOT_SUBSCRIPTION_REWARD_PAISE);
    assert_eq!(recipient_id, bot_id); // bot_id is used directly as recipient_id
    assert_eq!(pt, &token);
}

// GET /transactions?recipient_id returns the transaction for the recipient
#[tokio::test]
async fn test_get_user_transactions_after_grant() {
    let _db_guard = TestDbGuard::new();
    let token = format!("token_{}", uuid::Uuid::new_v4());

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&token, "bot_abc")).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let req = Request::builder()
        .method("GET")
        .uri("/transactions?recipient_id=bot_abc")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    let txns = body["data"].as_array().unwrap();
    assert_eq!(txns.len(), 1);
    assert_eq!(txns[0]["user_id"], "mock-user-id");
    assert_eq!(txns[0]["amount_paise"], BOT_SUBSCRIPTION_REWARD_PAISE);
    assert_eq!(txns[0]["recipient_id"], "bot_abc");
    assert_eq!(txns[0]["transaction_type"], "BotSubscriptionReward");
}

// GET /transactions?recipient_id returns empty list when no transactions exist
#[tokio::test]
async fn test_get_user_transactions_empty() {
    let _db_guard = TestDbGuard::new();

    let app = create_test_app().await;
    let req = Request::builder()
        .method("GET")
        .uri("/transactions?recipient_id=unknown-recipient")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body["data"].as_array().unwrap().len(), 0);
}

// GET /transactions/balance?recipient_id reflects accumulated rewards
#[tokio::test]
async fn test_balance_accumulates_across_grants() {
    let _db_guard = TestDbGuard::new();
    let bot_id = "bot_balance_test";

    // Two different users subscribe to the same bot
    for _ in 0..2 {
        let token = format!("token_{}", uuid::Uuid::new_v4());
        let app = create_test_app().await;
        let res = post_grant(app, &grant_request(&token, bot_id)).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    let app = create_test_app().await;
    let req = Request::builder()
        .method("GET")
        .uri("/transactions/balance?recipient_id=bot_balance_test")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(
        body["data"]["balance_paise"],
        BOT_SUBSCRIPTION_REWARD_PAISE * 2
    );
    assert_eq!(body["data"]["balance_rupees"], 18.0); // 2 × 9 rupees
}

// GET /transactions/balance?recipient_id returns 0 for a bot with no rewards
#[tokio::test]
async fn test_balance_zero_for_unrewarded_bot() {
    let _db_guard = TestDbGuard::new();

    let app = create_test_app().await;
    let req = Request::builder()
        .method("GET")
        .uri("/transactions/balance?recipient_id=bot_with_no_subs")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body["data"]["balance_paise"], 0);
    assert_eq!(body["data"]["balance_rupees"], 0.0);
}
