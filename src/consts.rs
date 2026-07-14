pub static YRAL_PRO_CREDIT_ALLOTMENT: u32 = 30;
pub const BOT_SUBSCRIPTION_REWARD_PAISE: i64 = 900; // 9 rupees
pub const IMAGE_UNLOCK_REWARD_PAISE: i64 = 900; // PLACEHOLDER — confirm payout with product

/// Store product ID for the per-image unlock consumable.
/// The local Google Play mock hardcodes "mock-product-id" and cannot echo the
/// request, so the local build validates against that instead.
#[cfg(not(feature = "local"))]
pub const IMAGE_UNLOCK_PRODUCT_ID: &str = "image_unlock";
#[cfg(feature = "local")]
pub const IMAGE_UNLOCK_PRODUCT_ID: &str = "mock-product-id";

/// Product IDs allowed to grant bot chat access. Keeps a cheap image-unlock
/// purchase from being replayed for chat access (and vice versa).
#[cfg(not(feature = "local"))]
pub const CHAT_ACCESS_PRODUCT_IDS: &[&str] = &["daily_chat"];
#[cfg(feature = "local")]
pub const CHAT_ACCESS_PRODUCT_IDS: &[&str] = &["mock-product-id", "ios-chat-product"];

// PLACEHOLDER amounts — confirm payout split with product before launch.
pub const BOT_SUBSCRIPTION_INITIAL_REWARD_PAISE: i64 = 900; // ₹9 intro period
pub const BOT_SUBSCRIPTION_RENEWAL_REWARD_PAISE: i64 = 6900; // ₹69/week renewal

/// Real store prefix for per-bot subscription products, shared by
/// Play Console / App Store Connect (one product per bot, e.g. "bot_sub_<bot>").
/// Unlike `BOT_SUBSCRIPTION_PRODUCT_PREFIX` this is never aliased by the local
/// mock, so endpoints that must *reject* bot subscription products (e.g.
/// /google/verify) can check it in every build.
pub const BOT_SUBSCRIPTION_STORE_PREFIX: &str = "bot_sub";

/// Per-bot auto-renewable subscription products share this prefix in
/// Play Console / App Store Connect (one product per bot, e.g. "bot_sub_<bot>").
/// The local mocks hardcode "mock-product-id", so the local build matches that.
#[cfg(not(feature = "local"))]
pub const BOT_SUBSCRIPTION_PRODUCT_PREFIX: &str = BOT_SUBSCRIPTION_STORE_PREFIX;
#[cfg(feature = "local")]
pub const BOT_SUBSCRIPTION_PRODUCT_PREFIX: &str = "mock-product-id";

pub fn is_bot_subscription_product(product_id: &str) -> bool {
    product_id.starts_with(BOT_SUBSCRIPTION_PRODUCT_PREFIX)
}
