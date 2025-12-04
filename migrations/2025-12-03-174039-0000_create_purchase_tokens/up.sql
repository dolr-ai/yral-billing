CREATE TABLE purchase_tokens (
    id VARCHAR(36) PRIMARY KEY,
    user_id VARCHAR(255) NOT NULL,
    purchase_token TEXT NOT NULL,
    status VARCHAR(50) NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expiry_at TIMESTAMP NOT NULL,
    INDEX idx_user_id (user_id),
    INDEX idx_purchase_token (purchase_token(255)),
    INDEX idx_status (status)
);
