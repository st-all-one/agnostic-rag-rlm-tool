-- Migration 012: Add summaries table (hierarchical)
CREATE TABLE IF NOT EXISTS summaries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    buffer_id INTEGER NOT NULL,
    content TEXT NOT NULL,
    scope TEXT NOT NULL CHECK(scope IN ('file', 'module', 'project')),
    source_chunk_ids TEXT,       -- JSON array of parent chunk IDs
    source_hash TEXT,            -- hash of all source chunks' content
    confidence REAL DEFAULT 0.0, -- 0.0-1.0
    version INTEGER DEFAULT 1,
    tokens INTEGER,
    parent_summary_id INTEGER,   -- for module/project: ID of parent summary
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now')),
    FOREIGN KEY (buffer_id) REFERENCES buffers(id) ON DELETE CASCADE,
    FOREIGN KEY (parent_summary_id) REFERENCES summaries(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_summaries_buffer ON summaries(buffer_id);
CREATE INDEX IF NOT EXISTS idx_summaries_scope ON summaries(scope);
CREATE INDEX IF NOT EXISTS idx_summaries_source_hash ON summaries(source_hash);
CREATE INDEX IF NOT EXISTS idx_summaries_parent ON summaries(parent_summary_id);
