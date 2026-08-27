-- Plan 021: temporal / versioning metadata for derivative tables.
--
-- Introduces the temporal columns that the server temporal-knowledge epics
-- build on (immutable chunks, authorship propagation, superseding,
-- time-travel):
--   * version        — monotonic per-row revision counter (starts at 1)
--   * is_active      — 0/1 soft-delete flag; existing rows backfill to 1 via
--                      the DEFAULT, so no separate UPDATE is required
--   * superseded_by  — rowid of the newer revision that replaced this one
--   * epoch          — project epoch at write time (drift / time-travel)
--   * created_by     — agent username (populated later by issue 786a)
--   * model          — LLM that produced the row (populated later by 786a)
--
-- Existing rows are backfilled by the DEFAULT clauses on the NOT NULL columns
-- (no data rewrite). `explorations` already carries `created_by`, `model` and
-- `epoch_created`, so those are intentionally NOT duplicated here — only the
-- missing temporal columns are added.
--
-- Partial indices over the scoping columns (project / buffer_id / file_path)
-- restricted to `is_active = 1` let readers cheaply filter live rows without
-- scanning superseded history.

-- chunks
ALTER TABLE chunks ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE chunks ADD COLUMN is_active INTEGER NOT NULL DEFAULT 1;
ALTER TABLE chunks ADD COLUMN superseded_by INTEGER;
ALTER TABLE chunks ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0;
ALTER TABLE chunks ADD COLUMN created_by TEXT;
ALTER TABLE chunks ADD COLUMN model TEXT;
CREATE INDEX IF NOT EXISTS idx_chunks_active ON chunks (buffer_id, file_path) WHERE is_active = 1;

-- qa_cache (model already present in 016)
ALTER TABLE qa_cache ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE qa_cache ADD COLUMN is_active INTEGER NOT NULL DEFAULT 1;
ALTER TABLE qa_cache ADD COLUMN superseded_by INTEGER;
ALTER TABLE qa_cache ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0;
ALTER TABLE qa_cache ADD COLUMN created_by TEXT;
CREATE INDEX IF NOT EXISTS idx_qa_cache_active ON qa_cache (project, buffer_id) WHERE is_active = 1;

-- rlm_nodes (model already present in 018)
ALTER TABLE rlm_nodes ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE rlm_nodes ADD COLUMN is_active INTEGER NOT NULL DEFAULT 1;
ALTER TABLE rlm_nodes ADD COLUMN superseded_by INTEGER;
ALTER TABLE rlm_nodes ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0;
ALTER TABLE rlm_nodes ADD COLUMN created_by TEXT;
CREATE INDEX IF NOT EXISTS idx_rlm_nodes_active ON rlm_nodes (project, level, subject) WHERE is_active = 1;

-- explorations (created_by, model, epoch_created already present in 019)
ALTER TABLE explorations ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE explorations ADD COLUMN is_active INTEGER NOT NULL DEFAULT 1;
ALTER TABLE explorations ADD COLUMN superseded_by INTEGER;
CREATE INDEX IF NOT EXISTS idx_explorations_active ON explorations (project) WHERE is_active = 1;
