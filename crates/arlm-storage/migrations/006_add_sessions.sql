-- version 6
-- Sessões multi-turn
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    project_name TEXT NOT NULL,
    title TEXT NOT NULL,
    created_at INTEGER DEFAULT (unixepoch())
) STRICT;

-- Contextos versionados
CREATE TABLE IF NOT EXISTS session_contexts (
    session_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    payload TEXT NOT NULL,
    created_at INTEGER DEFAULT (unixepoch()),
    PRIMARY KEY (session_id, version),
    FOREIGN KEY (session_id) REFERENCES sessions(id)
) STRICT;

-- Histórico de sessões
CREATE TABLE IF NOT EXISTS session_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    query TEXT NOT NULL,
    result TEXT,
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (session_id) REFERENCES sessions(id)
) STRICT;
