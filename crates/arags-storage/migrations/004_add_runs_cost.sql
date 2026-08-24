-- version 4
-- Runs com custo agregado (accountability por agente)
CREATE TABLE IF NOT EXISTS runs (
    id TEXT PRIMARY KEY,
    task TEXT NOT NULL,
    backend TEXT,
    mode TEXT,
    status TEXT,
    agent TEXT,
    started_at INTEGER,
    finished_at INTEGER,
    duration_ms INTEGER,
    total_cost REAL DEFAULT 0,
    total_tokens INTEGER DEFAULT 0,
    total_calls INTEGER DEFAULT 0,
    max_depth INTEGER,
    nodes_visited INTEGER,
    partial_answer TEXT,
    error TEXT
) STRICT;

-- Uso por modelo dentro de uma run
CREATE TABLE IF NOT EXISTS run_model_usage (
    run_id TEXT NOT NULL,
    model TEXT NOT NULL,
    calls INTEGER DEFAULT 0,
    input_tokens INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    cost REAL DEFAULT 0,
    PRIMARY KEY (run_id, model),
    FOREIGN KEY (run_id) REFERENCES runs(id)
) STRICT;

-- Custo por nó (análise granular)
CREATE TABLE IF NOT EXISTS node_calls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    depth INTEGER,
    node_type TEXT,
    model TEXT,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cost REAL,
    duration_ms INTEGER,
    status TEXT,
    error TEXT,
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (run_id) REFERENCES runs(id)
) STRICT;

-- View de relatório de custo por agente
CREATE VIEW IF NOT EXISTS agent_cost_report AS
SELECT
    agent,
    COUNT(*) as runs,
    SUM(total_cost) as total_cost,
    SUM(total_tokens) as total_tokens,
    AVG(duration_ms) as avg_duration_ms
FROM runs
GROUP BY agent;

-- Índices
CREATE INDEX IF NOT EXISTS idx_runs_agent ON runs(agent);
CREATE INDEX IF NOT EXISTS idx_runs_status ON runs(status);
CREATE INDEX IF NOT EXISTS idx_runs_started ON runs(started_at);
CREATE INDEX IF NOT EXISTS idx_node_calls_run ON node_calls(run_id);
