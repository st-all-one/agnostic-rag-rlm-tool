-- Plan 023: exploration review gate (quality gate borrowed from RLM).
--
-- `[exploration] require_review` puts non-admin maps into `pending_review`
-- until an admin approves (→ fresh) or rejects (→ retired). SQLite cannot
-- alter a CHECK constraint in place, so the table is rebuilt with the new
-- allowed status and all data carried over.

CREATE TABLE IF NOT EXISTS explorations_new (
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
                        CHECK (status IN ('fresh','stale','retired','pending_review')),
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

INSERT INTO explorations_new
SELECT id, exploration_id, project, buffer_id, goal, body, summary, created_by, model,
       template_version, epoch_created, status, stale_reason, confirmed, contradicted,
       access_count, token_count, created_at, updated_at, last_accessed_at, retired_at,
       retired_by
FROM explorations;

DROP TABLE explorations;
ALTER TABLE explorations_new RENAME TO explorations;

CREATE INDEX IF NOT EXISTS idx_explorations_project ON explorations (project, status);
CREATE INDEX IF NOT EXISTS idx_explorations_buffer ON explorations (buffer_id);

-- Recreate FTS sync triggers (dropped with the old table).
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
