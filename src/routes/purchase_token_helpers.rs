use crate::{
    error::{AppError, AppResult},
    types::{google_play_subscription_state, GooglePlaySubscriptionResponse},
};

pub fn verify_subcription_response_for_active_status(
    subscription_response: &GooglePlaySubscriptionResponse,
) -> AppResult<()> {
    match subscription_response.subscription_state.as_str() {
        google_play_subscription_state::SUBSCRIPTION_STATE_ACTIVE => Ok(()),
        google_play_subscription_state::SUBSCRIPTION_STATE_CANCELED => {
            Err(AppError::SubscriptionCanceled)
        }
        google_play_subscription_state::SUBSCRIPTION_STATE_IN_GRACE_PERIOD => Ok(()),
        google_play_subscription_state::SUBSCRIPTION_STATE_ON_HOLD => {
            Err(AppError::SubscriptionOnHold)
        }
        google_play_subscription_state::SUBSCRIPTION_STATE_PAUSED => {
            Err(AppError::SubscriptionPaused)
        }
        google_play_subscription_state::SUBSCRIPTION_STATE_EXPIRED => {
            Err(AppError::SubscriptionExpired)
        }
        _ => Err(AppError::SubscriptionInvalidState),
    }
}
