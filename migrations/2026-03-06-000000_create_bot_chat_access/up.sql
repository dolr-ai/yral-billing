CREATE TABLE bot_chat_access (
    id VARCHAR(36) PRIMARY KEY NOT NULL,
    purchase_token TEXT NOT NULL UNIQUE,
    user_id VARCHAR(255) NOT NULL,
    bot_id VARCHAR(255) NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'consume_pending',
    granted_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL
);

CREATE INDEX idx_bot_chat_access_user_bot ON bot_chat_access (user_id, bot_id);
CREATE INDEX idx_bot_chat_access_expires_at ON bot_chat_access (expires_at);
CREATE INDEX idx_bot_chat_access_status ON bot_chat_access (status);
