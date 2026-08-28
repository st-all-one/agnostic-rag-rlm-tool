-- Audit log (append-only) of key data-plane actions (issue agnostic-rag-rlm-tool-7222).
CREATE TABLE IF NOT EXISTS audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project TEXT,
    username TEXT NOT NULL,
    action TEXT NOT NULL,
    target TEXT,
    detail TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX IF NOT EXISTS idx_audit_log_username_created
    ON audit_log (username, created_at);

CREATE INDEX IF NOT EXISTS idx_audit_log_project_created
    ON audit_log (project, created_at);
