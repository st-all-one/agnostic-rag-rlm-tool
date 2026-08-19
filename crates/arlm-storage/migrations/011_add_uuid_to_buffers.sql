-- version 11: Add UUID column to buffers for global project identification
ALTER TABLE buffers ADD COLUMN uuid TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_buffers_uuid ON buffers(uuid);
