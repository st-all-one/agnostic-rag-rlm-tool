-- Migration 014: FTS5 index over summaries for dual-layer search (Plan 016, gap #1/#2).
-- Mirrors chunks_fts but for the summaries table.

CREATE VIRTUAL TABLE IF NOT EXISTS summaries_fts USING fts5(
    content,
    tokenize='porter unicode61',
    detail='column'
);

-- Populate from any summaries already present.
INSERT INTO summaries_fts(rowid, content)
SELECT id, content FROM summaries WHERE id NOT IN (SELECT rowid FROM summaries_fts);

-- Keep the FTS index in sync with the summaries table.
CREATE TRIGGER IF NOT EXISTS summaries_fts_ai AFTER INSERT ON summaries BEGIN
    INSERT INTO summaries_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER IF NOT EXISTS summaries_fts_ad AFTER DELETE ON summaries BEGIN
    DELETE FROM summaries_fts WHERE rowid = old.id;
END;

CREATE TRIGGER IF NOT EXISTS summaries_fts_au AFTER UPDATE ON summaries BEGIN
    DELETE FROM summaries_fts WHERE rowid = old.id;
    INSERT INTO summaries_fts(rowid, content) VALUES (new.id, new.content);
END;
