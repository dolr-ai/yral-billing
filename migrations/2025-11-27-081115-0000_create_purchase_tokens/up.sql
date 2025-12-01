
-- SQLite migration for purchase_tokens table

CREATE TABLE purchase_tokens (
	id TEXT PRIMARY KEY NOT NULL,
	user_id TEXT NOT NULL,
	purchase_token TEXT NOT NULL UNIQUE,
	status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'acknowledged', 'expired')),
	created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
