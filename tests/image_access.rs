use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use diesel::prelude::*;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use tower::ServiceExt; // for `oneshot`
use uuid;
use yral_billing::routes::image_access::{check_image_access_batch, grant_image_access};
use yral_billing::routes::transactions::get_balance;
use yral_billing::types::{BotChatAccessStatus, GrantImageAccessRequest, PurchaseSource};
use yral_billing::AppState;

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

async fn create_test_app() -> Router {
    let app_state = AppState::new().await;
    Router::new()
        .route(
            "/google/image-access/grant",
            axum::routing::post(grant_image_access),
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
        let test_db = format!("./test_image_{}.db", uuid::Uuid::new_v4());
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

fn grant_request(purchase_token: &str, image_id: &str) -> GrantImageAccessRequest {
    GrantImageAccessRequest {
        package_name: "com.example".to_string(),
        product_id: "mock-product-id".to_string(),
        purchase_token: purchase_token.to_string(),
        image_id: image_id.to_string(),
        bot_id: "bot_abc".to_string(),
    }
}

async fn post_grant(app: Router, payload: &GrantImageAccessRequest) -> axum::response::Response {
    let req = Request::builder()
        .method("POST")
        .uri("/google/image-access/grant")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(payload).unwrap()))
        .unwrap();
    app.oneshot(req).await.unwrap()
}

async fn post_check_batch(
    app: Router,
    user_id: &str,
    image_ids: &[&str],
) -> axum::response::Response {
    let payload = serde_json::json!({ "user_id": user_id, "image_ids": image_ids });
    let req = Request::builder()
        .method("POST")
        .uri("/image-access/check-batch")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();
    app.oneshot(req).await.unwrap()
}

async fn body_json(res: axum::response::Response) -> serde_json::Value {
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body_bytes).unwrap()
}

// Grant succeeds, then check-batch reports true for that image and false for another
#[tokio::test]
async fn test_grant_image_access_success_and_check() {
    let _db_guard = TestDbGuard::new();
    let token = format!("token_{}", uuid::Uuid::new_v4());
    let image_id = "msg-1:https://example.com/img.png";

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&token, image_id)).await;
    assert_eq!(res.status(), StatusCode::OK);

    // Mock returns obfuscated_external_account_id = "mock-user-id"
    let app = create_test_app().await;
    let res = post_check_batch(app, "mock-user-id", &[image_id, "msg-1:other"]).await;
    assert_eq!(res.status(), StatusCode::OK);

    let response = body_json(res).await;
    assert_eq!(response["data"]["access"][image_id], true);
    assert_eq!(response["data"]["access"]["msg-1:other"], false);
}

// Calling grant twice with same token + same image is idempotent — returns 200
#[tokio::test]
async fn test_grant_image_access_idempotent() {
    let _db_guard = TestDbGuard::new();
    let token = format!("token_{}", uuid::Uuid::new_v4());
    let payload = grant_request(&token, "msg-1:0");

    let app = create_test_app().await;
    let res = post_grant(app, &payload).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let res = post_grant(app, &payload).await;
    assert_eq!(res.status(), StatusCode::OK);
}

// Same token used for a different image returns 400 TokenAlreadyUsed
#[tokio::test]
async fn test_grant_image_access_different_image_rejected() {
    let _db_guard = TestDbGuard::new();
    let token = format!("token_{}", uuid::Uuid::new_v4());

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&token, "msg-1:0")).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&token, "msg-2:0")).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

// A product id other than the image unlock product is rejected before any store call
#[tokio::test]
async fn test_grant_image_access_wrong_product_id_rejected() {
    let _db_guard = TestDbGuard::new();
    let app = create_test_app().await;

    let mut payload = grant_request(&format!("token_{}", uuid::Uuid::new_v4()), "msg-1:0");
    payload.product_id = "daily_chat".to_string();

    let res = post_grant(app, &payload).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

// Empty image_id is rejected
#[tokio::test]
async fn test_grant_image_access_empty_image_id_rejected() {
    let _db_guard = TestDbGuard::new();
    let app = create_test_app().await;

    let payload = grant_request(&format!("token_{}", uuid::Uuid::new_v4()), "");
    let res = post_grant(app, &payload).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

// ConsumePending retry does not double-insert the creator reward
#[tokio::test]
async fn test_grant_image_access_reward_not_duplicated_on_retry() {
    use yral_billing::schema::image_access::dsl;

    let db_guard = TestDbGuard::new();
    let token = format!("token_{}", uuid::Uuid::new_v4());
    let payload = grant_request(&token, "msg-1:0");

    let app = create_test_app().await;
    let res = post_grant(app, &payload).await;
    assert_eq!(res.status(), StatusCode::OK);

    // Simulate a crash between consume and activation
    let mut conn = SqliteConnection::establish(db_guard.db_path()).unwrap();
    diesel::update(dsl::image_access.filter(dsl::purchase_token.eq(&token)))
        .set(dsl::status.eq(BotChatAccessStatus::ConsumePending))
        .execute(&mut conn)
        .unwrap();

    let app = create_test_app().await;
    let res = post_grant(app, &payload).await;
    assert_eq!(res.status(), StatusCode::OK);

    let app = create_test_app().await;
    let req = Request::builder()
        .method("GET")
        .uri("/transactions/balance?recipient_id=bot_abc")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let response = body_json(res).await;
    assert_eq!(response["data"]["balance_paise"], 900);
}

// Canceled rows do not grant access
#[tokio::test]
async fn test_check_image_access_canceled() {
    use yral_billing::schema::image_access::dsl;

    let db_guard = TestDbGuard::new();
    let token = format!("token_{}", uuid::Uuid::new_v4());
    let image_id = "msg-1:0";

    let app = create_test_app().await;
    let res = post_grant(app, &grant_request(&token, image_id)).await;
    assert_eq!(res.status(), StatusCode::OK);

    let mut conn = SqliteConnection::establish(db_guard.db_path()).unwrap();
    let now = chrono::Utc::now().naive_utc();
    diesel::update(dsl::image_access.filter(dsl::purchase_token.eq(&token)))
        .set((
            dsl::status.eq(BotChatAccessStatus::Canceled),
            dsl::updated_at.eq(now),
        ))
        .execute(&mut conn)
        .unwrap();

    let app = create_test_app().await;
    let res = post_check_batch(app, "mock-user-id", &[image_id]).await;
    assert_eq!(res.status(), StatusCode::OK);

    let response = body_json(res).await;
    assert_eq!(response["data"]["access"][image_id], false);
}

// Access rows are source-agnostic in the check: a Google row inserted directly still matches
#[tokio::test]
async fn test_check_image_access_ignores_purchase_source() {
    use yral_billing::model::ImageAccess;
    use yral_billing::schema::image_access;

    let db_guard = TestDbGuard::new();
    let image_id = "msg-9:0";

    let mut conn = SqliteConnection::establish(db_guard.db_path()).unwrap();
    let mut row = ImageAccess::new(
        PurchaseSource::Apple,
        format!("txn_{}", uuid::Uuid::new_v4()),
        "mock-user-id".to_string(),
        "bot_abc".to_string(),
        image_id.to_string(),
    );
    row.status = BotChatAccessStatus::Active;
    diesel::insert_into(image_access::table)
        .values(&row)
        .execute(&mut conn)
        .unwrap();

    let app = create_test_app().await;
    let res = post_check_batch(app, "mock-user-id", &[image_id]).await;
    assert_eq!(res.status(), StatusCode::OK);

    let response = body_json(res).await;
    assert_eq!(response["data"]["access"][image_id], true);
}

// Batch limits: empty list and >200 unique ids are rejected
#[tokio::test]
async fn test_check_image_access_batch_limits() {
    let _db_guard = TestDbGuard::new();

    let app = create_test_app().await;
    let res = post_check_batch(app, "mock-user-id", &[]).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let too_many: Vec<String> = (0..201).map(|i| format!("msg-{i}:0")).collect();
    let too_many_refs: Vec<&str> = too_many.iter().map(String::as_str).collect();
    let app = create_test_app().await;
    let res = post_check_batch(app, "mock-user-id", &too_many_refs).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
