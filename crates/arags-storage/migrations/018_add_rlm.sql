-- RLM: Recursive Language Model summaries (hierarchical, volunteer-processed).
--
-- `rlm_nodes` stores recursive summaries as a SEPARATE dataset (same pattern as
-- qa_cache): L1 = one summary per file (from its chunks); L2 = one summary per
-- theme/module (from related L1 file summaries); L3 = project overview (from
-- L2 theme summaries). Each node records provenance (source_hashes for
-- invalidation), attribution (volunteer_username + model) and a review gate:
-- only approved nodes are searchable.
--
-- `rlm_edges` is the provenance graph: an L1 node points at chunk ids; an L2
-- node points at its L1 node ids; L3 at its L2 ids. Invalidation walks this
-- graph bottom-up with per-level tolerance (server-side policy).
--
-- `rlm_jobs` is the distributed work queue: volunteers claim pending jobs with
-- a lease (default 500s, client-configurable); if a claimed job's source data
-- changes the server cancels it (worker observes generation mismatch) and a
-- replacement job is enqueued with elevated priority. Expired leases are
-- requeued so no work unit is lost or double-assigned while locked.

CREATE TABLE IF NOT EXISTS rlm_nodes (
    id                INTEGER PRIMARY KEY,
    node_id           TEXT NOT NULL UNIQUE,        -- UUIDv7, stable summary id
    buffer_id         INTEGER,                     -- scoping by project/buffer
    project           TEXT NOT NULL,
    level             INTEGER NOT NULL,            -- 1=file, 2=theme, 3=project
    subject           TEXT NOT NULL,               -- file path | theme name | project name
    summary_text      TEXT NOT NULL,
    source_hashes     TEXT,                        -- JSON array of content hashes (invalidation)
    model             TEXT,                        -- LLM that synthesized (metadata)
    volunteer_username TEXT,                       -- who processed (audit)
    template_version  TEXT,                        -- prompt template version used
    token_count       INTEGER NOT NULL DEFAULT 0,
    confidence        REAL NOT NULL DEFAULT 1.0,   -- decays over time / on staleness
    review_status     TEXT NOT NULL DEFAULT 'pending' CHECK (review_status IN ('pending','approved','rejected')),
    reviewed_by       TEXT,                        -- admin username (audit)
    reviewed_at       INTEGER,                     -- epoch ms (audit)
    access_count      INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    last_accessed_at  INTEGER NOT NULL,
    stale             INTEGER NOT NULL DEFAULT 0   -- 0/1
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_rlm_nodes_subject
    ON rlm_nodes (project, level, subject);
CREATE INDEX IF NOT EXISTS idx_rlm_nodes_buffer ON rlm_nodes (buffer_id);
CREATE INDEX IF NOT EXISTS idx_rlm_nodes_review ON rlm_nodes (project, review_status);
CREATE INDEX IF NOT EXISTS idx_rlm_nodes_stale ON rlm_nodes (stale, project);

-- FTS5 over summary + subject text for lexical search of summaries. Standalone
-- index; rowid mirrors rlm_nodes.id; kept in sync by triggers.
CREATE VIRTUAL TABLE IF NOT EXISTS rlm_fts USING fts5(
    subject, summary_text
);

CREATE TRIGGER IF NOT EXISTS rlm_fts_ai AFTER INSERT ON rlm_nodes BEGIN
    INSERT INTO rlm_fts (rowid, subject, summary_text)
    VALUES (new.id, new.subject, new.summary_text);
END;

CREATE TRIGGER IF NOT EXISTS rlm_fts_ad AFTER DELETE ON rlm_nodes BEGIN
    DELETE FROM rlm_fts WHERE rowid = old.id;
END;

CREATE TRIGGER IF NOT EXISTS rlm_fts_au AFTER UPDATE OF subject, summary_text ON rlm_nodes BEGIN
    DELETE FROM rlm_fts WHERE rowid = old.id;
    INSERT INTO rlm_fts (rowid, subject, summary_text)
    VALUES (old.id, new.subject, new.summary_text);
END;

-- Provenance graph: exactly one of child_node_id / chunk_id is set per edge.
CREATE TABLE IF NOT EXISTS rlm_edges (
    parent_id     INTEGER NOT NULL REFERENCES rlm_nodes(id) ON DELETE CASCADE,
    child_node_id INTEGER REFERENCES rlm_nodes(id) ON DELETE CASCADE,
    chunk_id      INTEGER REFERENCES chunks(id) ON DELETE CASCADE,
    PRIMARY KEY (parent_id, child_node_id, chunk_id)
);

CREATE INDEX IF NOT EXISTS idx_rlm_edges_child ON rlm_edges (child_node_id);
CREATE INDEX IF NOT EXISTS idx_rlm_edges_chunk ON rlm_edges (chunk_id);

-- Distributed work queue for volunteer processing.
CREATE TABLE IF NOT EXISTS rlm_jobs (
    id                INTEGER PRIMARY KEY,
    job_key           TEXT NOT NULL UNIQUE,      -- deterministic: "L<level>:<project>:<subject>"
    buffer_id         INTEGER,
    project           TEXT NOT NULL,
    level             INTEGER NOT NULL,
    subject           TEXT NOT NULL,
    payload           TEXT NOT NULL,             -- JSON: input refs (node_ids/chunk hashes/texts)
    generation        INTEGER NOT NULL DEFAULT 0,-- bumped on cancel/re-enqueue (cooperative cancel signal)
    status            TEXT NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending','claimed','done','failed','cancelled')),
    priority          INTEGER NOT NULL DEFAULT 5,-- lower = more urgent
    claimed_by        TEXT,                      -- volunteer username
    claimed_at        INTEGER,                   -- epoch ms
    lease_expires_at  INTEGER,                   -- epoch ms; claimed jobs past this requeue
    attempts          INTEGER NOT NULL DEFAULT 0,
    last_error        TEXT,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_rlm_jobs_claim ON rlm_jobs (status, priority, level);
CREATE INDEX IF NOT EXISTS idx_rlm_jobs_project ON rlm_jobs (project, status);
CREATE INDEX IF NOT EXISTS idx_rlm_jobs_lease ON rlm_jobs (lease_expires_at) WHERE status = 'claimed';
