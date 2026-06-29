CREATE TABLE bot_chat_access_new (
    id VARCHAR(36) PRIMARY KEY NOT NULL,
    purchase_source TEXT NOT NULL DEFAULT 'google',
    purchase_token TEXT NOT NULL,
    user_id VARCHAR(255) NOT NULL,
    bot_id VARCHAR(255) NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'consume_pending',
    granted_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL
);

INSERT INTO bot_chat_access_new (
    id,
    purchase_source,
    purchase_token,
    user_id,
    bot_id,
    status,
    granted_at,
    updated_at,
    expires_at
)
SELECT
    id,
    'google',
    purchase_token,
    user_id,
    bot_id,
    status,
    granted_at,
    updated_at,
    expires_at
FROM bot_chat_access;

DROP TABLE bot_chat_access;
ALTER TABLE bot_chat_access_new RENAME TO bot_chat_access;

CREATE UNIQUE INDEX idx_bot_chat_access_source_token
ON bot_chat_access (purchase_source, purchase_token);
CREATE INDEX idx_bot_chat_access_user_bot ON bot_chat_access (user_id, bot_id);
CREATE INDEX idx_bot_chat_access_expires_at ON bot_chat_access (expires_at);
CREATE INDEX idx_bot_chat_access_status ON bot_chat_access (status);

ALTER TABLE transactions
ADD COLUMN purchase_source TEXT NOT NULL DEFAULT 'google';

CREATE INDEX idx_transactions_source_token
ON transactions (purchase_source, purchase_token);

CREATE TABLE apple_app_account_tokens (
    id VARCHAR(36) PRIMARY KEY NOT NULL,
    app_account_token VARCHAR(36) NOT NULL UNIQUE,
    user_id VARCHAR(255) NOT NULL UNIQUE,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_apple_app_account_tokens_user_id
ON apple_app_account_tokens (user_id);
