CREATE TABLE transactions (
    id               VARCHAR(36)  NOT NULL PRIMARY KEY,
    user_id          VARCHAR(255) NOT NULL,
    transaction_type VARCHAR(50)  NOT NULL,
    amount_paise     BIGINT       NOT NULL,
    recipient_id     VARCHAR(255) NOT NULL,
    related_bot_id   VARCHAR(255) NOT NULL,
    purchase_token   TEXT         NOT NULL,
    created_at       TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_transactions_user_id       ON transactions (user_id);
CREATE INDEX idx_transactions_recipient_id  ON transactions (recipient_id);
