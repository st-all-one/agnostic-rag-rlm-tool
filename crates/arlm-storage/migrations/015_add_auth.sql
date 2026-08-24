-- version 15: auth & token management (plan 018)
--
-- Refresh tokens (auth_tokens) and short-lived session tokens (auth_sessions).
-- Named with an `auth_` prefix to avoid colliding with the pre-existing
-- multi-turn `sessions` table (migration 006).

CREATE TABLE IF NOT EXISTS auth_tokens (
    id          TEXT PRIMARY KEY,
    username    TEXT NOT NULL,
    role        TEXT NOT NULL CHECK (role IN ('admin', 'non_admin')),
    token_hash  TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL,
    created_by  TEXT NOT NULL,
    revoked     INTEGER NOT NULL DEFAULT 0,
    revoked_at  INTEGER,
    revoked_by  TEXT
) STRICT;

CREATE INDEX IF NOT EXISTS idx_auth_tokens_hash ON auth_tokens(token_hash);
CREATE INDEX IF NOT EXISTS idx_auth_tokens_username ON auth_tokens(username);

CREATE TABLE IF NOT EXISTS auth_sessions (
    id          TEXT PRIMARY KEY,
    token_id    TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL,
    FOREIGN KEY (token_id) REFERENCES auth_tokens(id)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_auth_sessions_token ON auth_sessions(token_id);
CREATE INDEX IF NOT EXISTS idx_auth_sessions_expires ON auth_sessions(expires_at);
