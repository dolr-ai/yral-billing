CREATE TABLE bot_subscriptions (
    id VARCHAR(36) PRIMARY KEY NOT NULL,
    purchase_source TEXT NOT NULL,
    purchase_token TEXT NOT NULL,
    user_id VARCHAR(255) NOT NULL,
    bot_id VARCHAR(255) NOT NULL,
    product_id TEXT NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    started_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL
);

CREATE UNIQUE INDEX idx_bot_subscriptions_source_token ON bot_subscriptions (purchase_source, purchase_token);
CREATE INDEX idx_bot_subscriptions_user_bot ON bot_subscriptions (user_id, bot_id);
CREATE INDEX idx_bot_subscriptions_status ON bot_subscriptions (status);
