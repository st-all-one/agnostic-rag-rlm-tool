-- Plan 022: Explorations — relational knowledge produced by explorer agents.
--
-- `explorations` stores dense, goal-driven maps of how code entities connect
-- (cross-module couplings, data flows, invariants), synthesized by an agent's
-- local LLM during exploration and persisted fire-and-forget (same pattern as
-- qa_cache). The server never runs an LLM: it validates, anchors, compresses,
-- embeds the summary into the dedicated `exploration_vectors` usearch index
-- (separate crate module; cosine space keyed by rowid) and serves with a
-- composite confidence score.
--
-- Staleness is anchor-based and deterministic: each cited file is stored in
-- `exploration_files` as (buffer_id, path, content_hash). When any anchor no
-- longer matches the current chunk hash for that buffer/path, the map becomes
-- `stale` with a granular `stale_reason` (JSON array of broken paths). The
-- flag is advisory; readers recheck anchors at hit time.
--
-- `project_epochs` is a monotone counter bumped on every index run that changed
-- data for a project. Maps record the epoch they were created at; drift
-- (current - created) feeds the confidence score even when no direct anchor
-- broke ("the world moved on").

CREATE TABLE IF NOT EXISTS explorations (
    id                INTEGER PRIMARY KEY,
    exploration_id    TEXT NOT NULL UNIQUE,        -- UUIDv7, stable map id
    project           TEXT NOT NULL,
    buffer_id         INTEGER,                     -- scoping buffer/project
    goal              TEXT NOT NULL,               -- objective that drove the exploration
    body              BLOB NOT NULL,               -- zstd-compressed markdown contract
    summary           TEXT NOT NULL,               -- short digest used for embedding
    created_by        TEXT NOT NULL,               -- agent username (audit/provenance)
    model             TEXT,                        -- LLM that produced the map (metadata)
    template_version  TEXT NOT NULL DEFAULT 'v1',  -- EXPLORATIONS.md contract version
    epoch_created     INTEGER NOT NULL DEFAULT 0,  -- project_epochs value at persist time
    status            TEXT NOT NULL DEFAULT 'fresh'
                        CHECK (status IN ('fresh','stale','retired')),
    stale_reason      TEXT,                        -- JSON array of broken anchor paths
    confirmed         INTEGER NOT NULL DEFAULT 0,  -- consumer verifications
    contradicted      INTEGER NOT NULL DEFAULT 0,  -- consumer contradictions
    access_count      INTEGER NOT NULL DEFAULT 0,
    token_count       INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    last_accessed_at  INTEGER NOT NULL,
    retired_at        INTEGER,                     -- epoch ms of retirement (audit)
    retired_by        TEXT                         -- who retired it (audit)
);

CREATE INDEX IF NOT EXISTS idx_explorations_project ON explorations (project, status);
CREATE INDEX IF NOT EXISTS idx_explorations_buffer ON explorations (buffer_id);

-- Anchor table: one row per cited/context file. `content_hash` is the chunk
-- content hash at persist time; staleness compares against the CURRENT hash
-- of (buffer_id, path) in chunks. role='cited' anchors invalidate; role=
-- 'context' anchors are provenance-only (do not invalidate by themselves).
CREATE TABLE IF NOT EXISTS exploration_files (
    exploration_rowid INTEGER NOT NULL REFERENCES explorations(id) ON DELETE CASCADE,
    buffer_id         INTEGER NOT NULL,
    path              TEXT NOT NULL,
    content_hash      TEXT NOT NULL,
    role              TEXT NOT NULL DEFAULT 'cited' CHECK (role IN ('cited','context')),
    PRIMARY KEY (exploration_rowid, path)
);

CREATE INDEX IF NOT EXISTS idx_exploration_files_buffer ON exploration_files (buffer_id);
CREATE INDEX IF NOT EXISTS idx_exploration_files_path ON exploration_files (buffer_id, path);

-- FTS5 over goal + summary text for lexical search. Standalone index; rowid
-- mirrors explorations.id; kept in sync by triggers.
CREATE VIRTUAL TABLE IF NOT EXISTS explorations_fts USING fts5(
    goal, summary
);

CREATE TRIGGER IF NOT EXISTS explorations_fts_ai AFTER INSERT ON explorations BEGIN
    INSERT INTO explorations_fts (rowid, goal, summary)
    VALUES (new.id, new.goal, new.summary);
END;

CREATE TRIGGER IF NOT EXISTS explorations_fts_ad AFTER DELETE ON explorations BEGIN
    DELETE FROM explorations_fts WHERE rowid = old.id;
END;

CREATE TRIGGER IF NOT EXISTS explorations_fts_au AFTER UPDATE OF goal, summary ON explorations BEGIN
    DELETE FROM explorations_fts WHERE rowid = old.id;
    INSERT INTO explorations_fts (rowid, goal, summary)
    VALUES (old.id, new.goal, new.summary);
END;

-- Monotone per-project epoch, bumped on every index run that changed data.
CREATE TABLE IF NOT EXISTS project_epochs (
    project    TEXT PRIMARY KEY,
    epoch      INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
);
