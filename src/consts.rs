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
