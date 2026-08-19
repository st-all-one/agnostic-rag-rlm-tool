-- version 8
-- Eventos persistidos (replay/auditoria)
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    event_json TEXT NOT NULL,
    created_at INTEGER DEFAULT (unixepoch())
) STRICT;

-- Índices
CREATE INDEX IF NOT EXISTS idx_events_run ON events(run_id);
CREATE INDEX IF NOT EXISTS idx_events_created ON events(created_at);
