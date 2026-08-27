-- Plan 023: inactive-chunk retention columns for immutable supersede writes
-- (issue `agnostic-rlm-rs-8dcc`).
--
-- Re-indexing no longer deletes chunks; it supersedes them (sets is_active = 0
-- and `superseded_by` to the new row). Retired rows keep their `chunk_texts`
-- / `chunk_entities` history until a maintenance purge reclaims them after a
-- configurable retention window.
--
-- `retired_at` records when a chunk was superseded (unix epoch seconds) so
-- `purge_inactive_chunks` can age out history. A partial index over the
-- retention columns lets the purge scan only retired rows.
--
-- Idempotent via the schema_version gate in `run_migrations` (runs once per
-- DB), mirroring the ALTER pattern in 021.

ALTER TABLE chunks ADD COLUMN retired_at INTEGER;
CREATE INDEX IF NOT EXISTS idx_chunks_retired ON chunks (retired_at) WHERE is_active = 0;
