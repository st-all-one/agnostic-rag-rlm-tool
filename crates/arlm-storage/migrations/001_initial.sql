-- version 1
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY,
    applied_at INTEGER DEFAULT (unixepoch())
);

-- Tabela principal de chunks (metadados)
CREATE TABLE IF NOT EXISTS chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    buffer_id INTEGER NOT NULL,
    file_path TEXT NOT NULL,
    offset_start INTEGER NOT NULL,
    offset_end INTEGER NOT NULL,
    line_start INTEGER NOT NULL,
    line_end INTEGER NOT NULL,
    hash BLOB NOT NULL,
    language TEXT,
    chunk_type TEXT,
    token_count INTEGER,
    status TEXT DEFAULT 'active',
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (buffer_id) REFERENCES buffers(id)
) STRICT;

-- Texto dos chunks (separado para não poluir cache de metadados)
CREATE TABLE IF NOT EXISTS chunk_texts (
    chunk_id INTEGER PRIMARY KEY REFERENCES chunks(id),
    content TEXT NOT NULL
) STRICT;

-- Buffers (projetos/diretórios indexados)
CREATE TABLE IF NOT EXISTS buffers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    path TEXT NOT NULL,
    total_chunks INTEGER DEFAULT 0,
    total_files INTEGER DEFAULT 0,
    embedding_model TEXT,
    embedding_dims INTEGER,
    last_indexed_at INTEGER,
    created_at INTEGER DEFAULT (unixepoch())
) STRICT;

-- Fila de tasks para dispatch/aggregate
CREATE TABLE IF NOT EXISTS tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    buffer_id INTEGER NOT NULL,
    chunk_id INTEGER,
    status TEXT DEFAULT 'pending',
    assigned_to TEXT,
    payload TEXT,
    result TEXT,
    created_at INTEGER DEFAULT (unixepoch()),
    started_at INTEGER,
    finished_at INTEGER,
    FOREIGN KEY (chunk_id) REFERENCES chunks(id)
) STRICT;

-- Resultados de subagentes
CREATE TABLE IF NOT EXISTS findings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL,
    chunk_id INTEGER,
    finding_type TEXT,
    content TEXT NOT NULL,
    confidence REAL,
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (task_id) REFERENCES tasks(id),
    FOREIGN KEY (chunk_id) REFERENCES chunks(id)
) STRICT;

-- Histórico de consultas
CREATE TABLE IF NOT EXISTS history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    buffer_id INTEGER,
    query TEXT NOT NULL,
    query_type TEXT,
    results_count INTEGER,
    duration_ms INTEGER,
    used_by TEXT,
    result_hash BLOB,
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (buffer_id) REFERENCES buffers(id)
) STRICT;

-- Padrões extraídos de análises
CREATE TABLE IF NOT EXISTS patterns (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    buffer_id INTEGER,
    pattern_type TEXT,
    name TEXT NOT NULL,
    description TEXT,
    examples TEXT,
    confidence REAL,
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (buffer_id) REFERENCES buffers(id)
) STRICT;

-- Índices
CREATE INDEX IF NOT EXISTS idx_chunks_file ON chunks(file_path);
CREATE INDEX IF NOT EXISTS idx_chunks_hash ON chunks(hash);
CREATE INDEX IF NOT EXISTS idx_chunks_buffer_hash ON chunks(buffer_id, hash);
CREATE INDEX IF NOT EXISTS idx_chunks_buffer_file ON chunks(buffer_id, file_path);
CREATE INDEX IF NOT EXISTS idx_tasks_buffer ON tasks(buffer_id);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_tasks_pending ON tasks(buffer_id) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS idx_findings_task ON findings(task_id);
CREATE INDEX IF NOT EXISTS idx_findings_chunk ON findings(chunk_id);
CREATE INDEX IF NOT EXISTS idx_history_buffer ON history(buffer_id);
CREATE INDEX IF NOT EXISTS idx_history_buffer_created ON history(buffer_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_patterns_buffer ON patterns(buffer_id);
