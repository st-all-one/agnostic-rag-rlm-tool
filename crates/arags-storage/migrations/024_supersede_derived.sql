-- Plan 021 follow-up: derived-record superseding (issue `agnostic-rag-rlm-tool-e210`).
--
-- `qa_cache`, `rlm_nodes` and `explorations` now keep history: storing a NEW
-- answer / RLM node / exploration map for the same subject no longer
-- UPDATEs-in-place. It inserts a fresh row (`version = old + 1`, `is_active =
-- 1`) and retires the previous active row (`is_active = 0`, `superseded_by =
-- new id`). The previous unique constraints blocked a second row per subject,
-- so they are replaced by *partial* unique indexes that permit unlimited
-- history rows but enforce exactly one ACTIVE row per subject.
--
-- `chunks` already did this in 8dcc/023; the three derived tables get the same
-- treatment here. Reads filter `is_active = 1` to see only the latest revision.
--
-- Idempotent via the schema_version gate in `run_migrations`.

-- qa_cache: unique was (project, question_hash); the supersede key also carries
-- buffer_id, so widen it and restrict to active rows only.
DROP INDEX IF EXISTS idx_qa_cache_project_question;
CREATE UNIQUE INDEX IF NOT EXISTS idx_qa_cache_active_unique
    ON qa_cache (project, buffer_id, question_hash) WHERE is_active = 1;

-- rlm_nodes: unique was (project, level, subject); keep that shape but limit to
-- the active revision.
DROP INDEX IF EXISTS idx_rlm_nodes_subject;
CREATE UNIQUE INDEX IF NOT EXISTS idx_rlm_nodes_subject_active
    ON rlm_nodes (project, level, subject) WHERE is_active = 1;
