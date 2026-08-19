-- version 5
-- Trajectória completa (aprendizado)
CREATE TABLE IF NOT EXISTS trajectories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_name TEXT NOT NULL,
    agent TEXT,
    root_json TEXT NOT NULL,
    task TEXT NOT NULL,
    task_hash TEXT,
    total_cost REAL DEFAULT 0,
    created_at INTEGER DEFAULT (unixepoch())
) STRICT;

-- Índices
CREATE INDEX IF NOT EXISTS idx_traj_project ON trajectories(project_name);
CREATE INDEX IF NOT EXISTS idx_traj_hash ON trajectories(task_hash);
