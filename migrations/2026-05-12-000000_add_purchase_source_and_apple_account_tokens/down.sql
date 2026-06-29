DROP TABLE apple_app_account_tokens;

DROP INDEX idx_transactions_source_token;

CREATE TABLE transactions_new (
    id               VARCHAR(36)  NOT NULL PRIMARY KEY,
    user_id          VARCHAR(255) NOT NULL,
    transaction_type VARCHAR(50)  NOT NULL,
    amount_paise     BIGINT       NOT NULL,
    recipient_id     VARCHAR(255) NOT NULL,
    purchase_token   TEXT         NOT NULL,
    created_at       TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO transactions_new (
    id,
    user_id,
    transaction_type,
    amount_paise,
    recipient_id,
    purchase_token,
    created_at
)
SELECT
    id,
    user_id,
    transaction_type,
    amount_paise,
    recipient_id,
    purchase_token,
    created_at
FROM transactions;

DROP TABLE transactions;
ALTER TABLE transactions_new RENAME TO transactions;

CREATE INDEX idx_transactions_user_id ON transactions (user_id);
CREATE INDEX idx_transactions_recipient_id ON transactions (recipient_id);

CREATE TABLE bot_chat_access_new (
    id VARCHAR(36) PRIMARY KEY NOT NULL,
    purchase_token TEXT NOT NULL UNIQUE,
    user_id VARCHAR(255) NOT NULL,
    bot_id VARCHAR(255) NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'consume_pending',
    granted_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL
);

INSERT INTO bot_chat_access_new (
    id,
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
    purchase_token,
    user_id,
    bot_id,
    status,
    granted_at,
    updated_at,
    expires_at
FROM bot_chat_access
WHERE purchase_source = 'google';

DROP TABLE bot_chat_access;
ALTER TABLE bot_chat_access_new RENAME TO bot_chat_access;

CREATE INDEX idx_bot_chat_access_user_bot ON bot_chat_access (user_id, bot_id);
CREATE INDEX idx_bot_chat_access_expires_at ON bot_chat_access (expires_at);
CREATE INDEX idx_bot_chat_access_status ON bot_chat_access (status);
