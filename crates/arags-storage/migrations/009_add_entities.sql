-- Entities extracted from chunks (deterministic, no LLM)
CREATE TABLE IF NOT EXISTS chunk_entities (
    chunk_id INTEGER NOT NULL,
    entity TEXT NOT NULL,
    PRIMARY KEY (chunk_id, entity),
    FOREIGN KEY (chunk_id) REFERENCES chunks(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_chunk_entities_entity ON chunk_entities(entity);

-- FTS5 contentless index over entities for BM25 search
CREATE VIRTUAL TABLE IF NOT EXISTS entities_fts USING fts5(
    entity,
    tokenize='porter unicode61'
);
