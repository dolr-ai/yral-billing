use crate::types::{
    one_time_product_notification_type, subscription_notification_type, DeveloperNotification,
    PubSubMessage,
};
use axum::{http::StatusCode, response::IntoResponse, Json};
use base64::prelude::*;
use serde_json;

pub async fn handle_rtdn_webhook(Json(payload): Json<PubSubMessage>) -> impl IntoResponse {
    println!("Received RTDN webhook: {:?}", payload);

    // Decode the base64 message data
    let decoded_data = match BASE64_STANDARD.decode(&payload.message.data) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Failed to decode base64 data: {}", e);
            return (StatusCode::BAD_REQUEST, "Invalid base64 data");
        }
    };

    // Parse the developer notification
    let notification_json = match String::from_utf8(decoded_data) {
        Ok(json_str) => json_str,
        Err(e) => {
            eprintln!("Failed to convert to UTF-8: {}", e);
            return (StatusCode::BAD_REQUEST, "Invalid UTF-8 data");
        }
    };

    let notification: DeveloperNotification = match serde_json::from_str(&notification_json) {
        Ok(notif) => notif,
        Err(e) => {
            eprintln!("Failed to parse notification JSON: {}", e);
            return (StatusCode::BAD_REQUEST, "Invalid notification format");
        }
    };

    // Process the notification
    match process_notification(&notification).await {
        Ok(_) => {
            println!(
                "Successfully processed notification for package: {}",
                notification.package_name
            );
            // HTTP 200 acknowledges the message to Pub/Sub
            (StatusCode::OK, "OK")
        }
        Err(e) => {
            eprintln!("Failed to process notification: {}", e);
            // HTTP 500 causes Pub/Sub to retry delivery
            // Consider returning 200 for permanent failures to avoid infinite retries
            (StatusCode::INTERNAL_SERVER_ERROR, "Processing failed")
        }
    }
}

async fn process_notification(
    notification: &DeveloperNotification,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!(
        "Processing notification for package: {}",
        notification.package_name
    );
    println!("Event time: {}", notification.event_time_millis);

    // Handle subscription notifications
    if let Some(sub_notification) = &notification.subscription_notification {
        handle_subscription_notification(sub_notification).await?;
    }

    // Handle one-time product notifications
    if let Some(product_notification) = &notification.one_time_product_notification {
        handle_one_time_product_notification(product_notification).await?;
    }

    // Handle test notifications
    if let Some(test_notification) = &notification.test_notification {
        handle_test_notification(test_notification).await?;
    }

    Ok(())
}

async fn handle_subscription_notification(
    notification: &crate::types::SubscriptionNotification,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let notification_type = notification.notification_type;
    let purchase_token = &notification.purchase_token;
    let subscription_id = &notification.subscription_id;

    println!(
        "Subscription notification - Type: {}, Token: {}, ID: {}",
        notification_type, purchase_token, subscription_id
    );

    match notification_type {
        subscription_notification_type::SUBSCRIPTION_PURCHASED => {
            println!("New subscription purchased");
            // TODO: Store subscription in database, send confirmation email, etc.
        }
        subscription_notification_type::SUBSCRIPTION_RENEWED => {
            println!("Subscription renewed");
            // TODO: Update subscription expiry, send renewal confirmation
        }
        subscription_notification_type::SUBSCRIPTION_CANCELED => {
            println!("Subscription canceled");
            // TODO: Mark subscription as canceled, handle cancellation logic
        }
        subscription_notification_type::SUBSCRIPTION_EXPIRED => {
            println!("Subscription expired");
            // TODO: Disable user access, send expiry notification
        }
        subscription_notification_type::SUBSCRIPTION_RECOVERED => {
            println!("Subscription recovered from account hold");
            // TODO: Restore user access
        }
        subscription_notification_type::SUBSCRIPTION_ON_HOLD => {
            println!("Subscription on hold");
            // TODO: Temporarily suspend user access
        }
        subscription_notification_type::SUBSCRIPTION_IN_GRACE_PERIOD => {
            println!("Subscription in grace period");
            // TODO: Send payment retry notification
        }
        subscription_notification_type::SUBSCRIPTION_RESTARTED => {
            println!("Subscription restarted");
            // TODO: Restore subscription, update expiry
        }
        subscription_notification_type::SUBSCRIPTION_PRICE_CHANGE_CONFIRMED => {
            println!("Subscription price change confirmed");
            // TODO: Update subscription pricing in database
        }
        subscription_notification_type::SUBSCRIPTION_DEFERRED => {
            println!("Subscription deferred");
            // TODO: Handle deferred billing
        }
        subscription_notification_type::SUBSCRIPTION_PAUSED => {
            println!("Subscription paused");
            // TODO: Pause user access, update status
        }
        subscription_notification_type::SUBSCRIPTION_PAUSE_SCHEDULE_CHANGED => {
            println!("Subscription pause schedule changed");
            // TODO: Update pause schedule
        }
        subscription_notification_type::SUBSCRIPTION_REVOKED => {
            println!("Subscription revoked");
            // TODO: Immediately revoke access, handle refund if applicable
        }
        _ => {
            println!(
                "Unknown subscription notification type: {}",
                notification_type
            );
        }
    }

    Ok(())
}

async fn handle_one_time_product_notification(
    notification: &crate::types::OneTimeProductNotification,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let notification_type = notification.notification_type;
    let purchase_token = &notification.purchase_token;
    let sku = &notification.sku;

    println!(
        "One-time product notification - Type: {}, Token: {}, SKU: {}",
        notification_type, purchase_token, sku
    );

    match notification_type {
        one_time_product_notification_type::ONE_TIME_PRODUCT_PURCHASED => {
            println!("One-time product purchased");
            // TODO: Grant product access, send confirmation
        }
        one_time_product_notification_type::ONE_TIME_PRODUCT_CANCELED => {
            println!("One-time product canceled");
            // TODO: Revoke product access, handle refund
        }
        _ => {
            println!(
                "Unknown one-time product notification type: {}",
                notification_type
            );
        }
    }

    Ok(())
}

async fn handle_test_notification(
    notification: &crate::types::TestNotification,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!(
        "Test notification received - Version: {}",
        notification.version
    );
    println!("This is a test notification from Google Play Console");

    // Test notifications don't require any special processing
    // They're just used to verify that your endpoint is working correctly

    Ok(())
}
