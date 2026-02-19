CREATE TABLE purchase_tokens (
    id VARCHAR(36) PRIMARY KEY NOT NULL,
    user_id VARCHAR(255) NOT NULL,
    purchase_token TEXT NOT NULL UNIQUE,
    status VARCHAR(50) NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expiry_at TIMESTAMP NOT NULL
);

CREATE INDEX idx_user_id ON purchase_tokens (user_id);
CREATE INDEX idx_status ON purchase_tokens (status);
