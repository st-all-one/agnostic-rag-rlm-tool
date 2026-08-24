-- version 7
-- Cache de resultados (dedup de subtasks)
CREATE TABLE IF NOT EXISTS result_cache (
    task_hash TEXT NOT NULL,
    project TEXT NOT NULL,
    result TEXT NOT NULL,
    run_id TEXT,
    created_at INTEGER DEFAULT (unixepoch()),
    hit_count INTEGER DEFAULT 1,
    PRIMARY KEY (task_hash, project),
    FOREIGN KEY (run_id) REFERENCES runs(id)
) STRICT;
