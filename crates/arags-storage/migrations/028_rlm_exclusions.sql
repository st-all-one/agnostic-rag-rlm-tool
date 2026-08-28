-- Issue `agnostic-rag-rlm-tool-f486`: per-generation-group volunteer exclusions.
--
-- When the cosine quorum exhausts a generation group (all candidates diverge
-- below `quorum_sim_threshold`) it re-fans the subject out to a NEW generation
-- group while excluding the volunteers that just diverged, so the same answers
-- are not regenerated. The exclusions are keyed by the generation group so a
-- later round can clear them. Idempotent: the runner skips this file once
-- `schema_version` has advanced past it; the `IF NOT EXISTS` guards make a
-- manual re-apply harmless too.

CREATE TABLE IF NOT EXISTS rlm_job_exclusions (
    generation_group_id INTEGER NOT NULL,
    volunteer           TEXT    NOT NULL,
    PRIMARY KEY (generation_group_id, volunteer)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS idx_rlm_job_exclusions_volunteer
    ON rlm_job_exclusions (volunteer);
