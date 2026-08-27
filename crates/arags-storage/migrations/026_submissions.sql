-- Cluster B keystone (issue `agnostic-rlm-rs-a5d7`): modelo de dados de
-- submissões candidatas + reputação de voluntários.
--
-- A tabela `submissions` guarda as respostas candidatas produzidas por
-- voluntários para um mesmo subject (nó RLM / exploração / qa). O decisor de
-- quórum (issues `6d97`/`64af`) consome essas linhas para computar o consenso
-- por similaridade de cosseno e fundir os candidatos de acordo em `accepted`.
-- O status cicla candidate -> accepted | rejected. A tabela `volunteer_trust`
-- acumula strikes e confiança para depriorizar/bloquear voluntários ruins.
--
-- Idempotent via o gate schema_version em `run_migrations`.

CREATE TABLE IF NOT EXISTS submissions (
    id INTEGER PRIMARY KEY,
    project TEXT NOT NULL,
    subject_type TEXT NOT NULL,     -- 'rlm_node' | 'exploration' | 'qa'
    subject_key TEXT NOT NULL,      -- id/chave do subject alvo da submissão
    candidate_text TEXT NOT NULL,
    candidate_by TEXT NOT NULL,     -- usuário voluntário (do refresh token)
    similarity REAL,                -- similaridade c/ aceito/consenso (no decisório)
    status TEXT NOT NULL DEFAULT 'candidate',  -- candidate | accepted | rejected
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    decided_at INTEGER,
    decided_by TEXT
);

CREATE INDEX IF NOT EXISTS idx_submissions_pending
    ON submissions (project, subject_type, subject_key, status);
CREATE INDEX IF NOT EXISTS idx_submissions_by_volunteer
    ON submissions (candidate_by, status);

CREATE TABLE IF NOT EXISTS volunteer_trust (
    username TEXT PRIMARY KEY,
    strikes INTEGER NOT NULL DEFAULT 0,
    trust_score REAL NOT NULL DEFAULT 1.0
);
