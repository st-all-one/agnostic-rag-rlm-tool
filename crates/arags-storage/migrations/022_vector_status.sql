-- Plan 023 / issue `agnostic-rag-rlm-tool-50ed`: vector-derivation failure tracking.
--
-- When an embedding or usearch vector insertion fails for a row, the server
-- must record a pending state in SQLite (instead of silently dropping the
-- row) so a later reconcile worker (issue `agnostic-rag-rlm-tool-36ae`) can
-- re-embed from the canonical text.
--
-- `chunks` already carries a free-form `status TEXT` column with no CHECK, so
-- the failure marker reuses it (`status = 'pending_vector'`). The dedicated
-- vector spaces (`rlm_nodes`, `explorations`, `qa_cache`) each get a NEW
-- `vector_status TEXT NOT NULL DEFAULT 'indexed'` column so we never violate
-- their existing constrained `status` CHECKs.
--
-- `ALTER TABLE ... ADD COLUMN` on a NOT NULL column with a DEFAULT is allowed
-- by SQLite without a table rebuild; existing rows backfill to 'indexed'.

ALTER TABLE rlm_nodes     ADD COLUMN vector_status TEXT NOT NULL DEFAULT 'indexed';
ALTER TABLE explorations  ADD COLUMN vector_status TEXT NOT NULL DEFAULT 'indexed';
ALTER TABLE qa_cache      ADD COLUMN vector_status TEXT NOT NULL DEFAULT 'indexed';
