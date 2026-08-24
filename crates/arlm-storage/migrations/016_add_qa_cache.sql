-- Plan 017: Semantic Query-Answer Cache (digestão sob demanda, no client).
--
-- `qa_cache` stores digested answers (synthesized client-side) keyed by
-- (project, question_hash). The server is deterministic: it embeds, searches,
-- and stores — it never runs an LLM. A separate `usearch` index
-- (`question_vectors`) holds the question embeddings in their own vector space
-- (cosine metric) so cache lookup never mixes with the chunk vector space.
--
-- Staleness is tracked by `source_hashes` (content hashes of the chunks that
-- produced the answer); the lifecycle hook marks rows `stale` when their
-- chunks change. Manual invalidation (admin-gated by plan 018) supports a soft
-- `Stale` mark and a hard `Delete`, plus a similarity radius over
-- `question_vectors` to invalidate a whole error chain at once.

CREATE TABLE IF NOT EXISTS qa_cache (
    id                INTEGER PRIMARY KEY,
    cache_id          TEXT NOT NULL UNIQUE,        -- UUIDv7, stable answer id (anti-drift)
    buffer_id         INTEGER,                     -- scoping by project/buffer
    project           TEXT NOT NULL,               -- redundant for fast lookup
    question_text     TEXT NOT NULL,               -- original question
    question_hash     TEXT NOT NULL,               -- exact-hit key (project, question_hash)
    answer_text       TEXT NOT NULL,               -- digested answer (from client)
    source_chunk_ids  TEXT,                        -- JSON array of chunk ids (provenance)
    source_hashes     TEXT,                        -- JSON array of chunk content hashes (invalidation)
    model             TEXT,                        -- LLM that synthesized (metadata, not a blocker)
    confidence        REAL NOT NULL DEFAULT 1.0,   -- decays to 0 when stale
    tier_snapshot     TEXT,                        -- JSON of thresholds used (reproducibility)
    token_count       INTEGER NOT NULL DEFAULT 0,
    access_count      INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    last_accessed_at  INTEGER NOT NULL,
    stale             INTEGER NOT NULL DEFAULT 0,  -- 0/1
    invalidated_at    INTEGER,                     -- epoch ms of manual invalidation (audit)
    invalidated_by    TEXT,                        -- username (audit)
    invalidated_reason TEXT                        -- reason (audit)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_qa_cache_project_question
    ON qa_cache (project, question_hash);
CREATE INDEX IF NOT EXISTS idx_qa_cache_cache_id ON qa_cache (cache_id);
CREATE INDEX IF NOT EXISTS idx_qa_cache_buffer ON qa_cache (buffer_id);
CREATE INDEX IF NOT EXISTS idx_qa_cache_stale ON qa_cache (stale, project);

-- FTS5 over question + answer text for lexical cache search. Maintained by
-- triggers (standalone index; rowid mirrors qa_cache.id).
CREATE VIRTUAL TABLE IF NOT EXISTS qa_cache_fts USING fts5(
    question_text, answer_text
);

-- Keep the FTS index in sync with qa_cache.
CREATE TRIGGER IF NOT EXISTS qa_cache_ai AFTER INSERT ON qa_cache BEGIN
    INSERT INTO qa_cache_fts (rowid, question_text, answer_text)
    VALUES (new.id, new.question_text, new.answer_text);
END;

CREATE TRIGGER IF NOT EXISTS qa_cache_ad AFTER DELETE ON qa_cache BEGIN
    DELETE FROM qa_cache_fts WHERE rowid = old.id;
END;

CREATE TRIGGER IF NOT EXISTS qa_cache_au AFTER UPDATE ON qa_cache BEGIN
    DELETE FROM qa_cache_fts WHERE rowid = old.id;
    INSERT INTO qa_cache_fts (rowid, question_text, answer_text)
    VALUES (old.id, new.question_text, new.answer_text);
END;
