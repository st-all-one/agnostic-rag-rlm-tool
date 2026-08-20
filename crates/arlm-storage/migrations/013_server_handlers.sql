-- version 13: server handler schema alignment
-- align `runs` and `sessions` with the columns used by arlm-server gRPC handlers

ALTER TABLE runs ADD COLUMN project TEXT;
ALTER TABLE runs ADD COLUMN model TEXT;

ALTER TABLE sessions ADD COLUMN updated_at INTEGER;

-- FTS5 full-text index over chunk texts (used by search/build_context, gap #15).
-- Bm25Search lazily created this in single-connection mode; the server runs in
-- pooled mode, so the table must exist before any handler runs a MATCH query.
CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
    content,
    tokenize='porter unicode61',
    detail='column'
);

CREATE INDEX IF NOT EXISTS idx_runs_project ON runs(project);
CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project_name);