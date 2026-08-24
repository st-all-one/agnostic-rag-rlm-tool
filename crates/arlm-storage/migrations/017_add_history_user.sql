-- version 17: history per-user scope
-- Add a `user` column to `history` so the server can record which authenticated
-- user issued each query (plan 019, section E). Existing rows get NULL `user`,
-- which the server backfills/populates from the auth token going forward.

ALTER TABLE history ADD COLUMN user TEXT;
