-- Plan `pl-783b` step 4 / issue `agnostic-rag-rlm-tool-d172`: re-digest de QA via
-- fila com lease (job pending_qa com preferred_user -> fallback voluntários ->
-- timeout 300s devolve ao queue p/ próximo ciclo).
--
-- A `pending_qa_jobs` queue lets a volunteer client claim a stale/derived QA
-- answer, re-digest it locally with its own LLM, and persist the fresh answer
-- via the existing StoreAnswer RPC. The original author is tried first
-- (`preferred_user`); any other volunteer may claim it after; an uncompleted
-- lease (default 300s) is reverted to `pending` by the maintenance ticker so a
-- crashed volunteer never strands the work unit.
--
-- Idempotent via the schema_version gate in `run_migrations`.

CREATE TABLE IF NOT EXISTS pending_qa_jobs (
    id INTEGER PRIMARY KEY,
    cache_id TEXT NOT NULL,          -- qa_cache stable id
    project TEXT NOT NULL,
    preferred_user TEXT,             -- original author; tried first
    status TEXT NOT NULL DEFAULT 'pending',  -- pending | leased | completed
    leased_by TEXT,
    leased_until INTEGER,            -- epoch seconds; NULL until leased
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    completed_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_pending_qa_status ON pending_qa_jobs(status);
CREATE INDEX IF NOT EXISTS idx_pending_qa_cache ON pending_qa_jobs(cache_id, status);
