CREATE TABLE image_access (
    id VARCHAR(36) PRIMARY KEY NOT NULL,
    purchase_source TEXT NOT NULL,
    purchase_token TEXT NOT NULL,
    user_id VARCHAR(255) NOT NULL,
    bot_id VARCHAR(255) NOT NULL,
    image_id TEXT NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'consume_pending',
    granted_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX idx_image_access_source_token ON image_access (purchase_source, purchase_token);
CREATE INDEX idx_image_access_user_image ON image_access (user_id, image_id);
CREATE INDEX idx_image_access_status ON image_access (status);
