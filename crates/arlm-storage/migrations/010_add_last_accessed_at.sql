-- version 8
-- Add last_accessed_at to chunks for salience decay tracking
ALTER TABLE chunks ADD COLUMN last_accessed_at INTEGER DEFAULT (unixepoch());

-- Backfill existing chunks: last_accessed_at = created_at
UPDATE chunks SET last_accessed_at = created_at WHERE last_accessed_at IS NULL;

-- Index for decay queries (order by freshness)
CREATE INDEX IF NOT EXISTS idx_chunks_last_accessed ON chunks(last_accessed_at DESC);
