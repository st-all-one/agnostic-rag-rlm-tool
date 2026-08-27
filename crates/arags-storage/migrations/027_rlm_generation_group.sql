-- RLM quorum fan-out (issue `agnostic-rlm-rs-6d97`, plan `pl-84c3` step 2):
-- a single subject may be fanned out to N independent volunteer job slots that
-- share a `generation_group_id`. Each slot is claimed independently (one slot
-- per volunteer per group), completed independently, and its candidate summary
-- is staged in `submissions`; the cosine quorum decides which consensus is
-- published as the live RLM node.
--
-- Idempotent via the `schema_version` gate in `run_migrations`.

ALTER TABLE rlm_jobs ADD COLUMN generation_group_id INTEGER;

CREATE INDEX IF NOT EXISTS idx_rlm_jobs_group
    ON rlm_jobs (generation_group_id);
