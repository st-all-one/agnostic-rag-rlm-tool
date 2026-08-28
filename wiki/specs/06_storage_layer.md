# Storage Layer — SQLite + usearch

## Visão Geral

O `arags-storage` gerencia toda persistência: SQLite para metadados/FTS5/estado, usearch para vetores. A separação permite que cada sistema seja especialista no que faz melhor.

```
┌──────────────────────────────────────────────────────────┐
│                    arags-storage                           │
│                                                          │
│  ┌──────────────────────┐  ┌──────────────────────────┐  │
│  │      SQLite          │  │       usearch            │  │
│  │  ┌────────────────┐  │  │  ┌────────────────────┐  │  │
│  │  │ chunks         │  │  │  │ vectors            │  │  │
│  │  │ chunk_texts    │  │  │  │ (chunk_id, vector, │  │  │
│  │  │ buffers        │  │  │  │  buffer_id)        │  │  │
│  │  │ tasks          │  │  │  └────────────────────┘  │  │
│  │  │ findings       │  │  │  ┌────────────────────┐  │  │
│  │  │ history        │  │  │  │ HNSW Index         │  │  │
│  │  │ patterns       │  │  │  │ (m=16, ef=200)     │  │  │
│  │  └────────────────┘  │  │  └────────────────────┘  │  │
│  │  ┌────────────────┐  │  │                          │  │
│  │  │ FTS5 (BM25)    │  │  │                          │  │
│  │  │ chunks_fts     │  │  │                          │  │
│  │  └────────────────┘  │  │                          │  │
│  └──────────────────────┘  └──────────────────────────┘  │
└──────────────────────────────────────────────────────────┘
```

## SQLite Schema

### Tabelas Principais

```sql
-- Metadados de chunks (leve, sem texto grande)
CREATE TABLE chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    buffer_id INTEGER NOT NULL,
    file_path TEXT NOT NULL,
    offset_start INTEGER NOT NULL,
    offset_end INTEGER NOT NULL,
    line_start INTEGER NOT NULL,
    line_end INTEGER NOT NULL,
    hash BLOB NOT NULL,          -- SHA256 para detectar mudanças
    language TEXT,                -- 'rust', 'python', etc.
    chunk_type TEXT,              -- 'function', 'class', 'paragraph'
    token_count INTEGER,
    status TEXT DEFAULT 'active', -- 'active', 'processing', 'done'
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (buffer_id) REFERENCES buffers(id)
) STRICT;

-- Texto dos chunks (separado para não poluir cache de metadados)
CREATE TABLE chunk_texts (
    chunk_id INTEGER PRIMARY KEY REFERENCES chunks(id),
    content TEXT NOT NULL          -- Comprimido com zstd (FTS5 recebe texto puro em separado)
) STRICT;

-- Buffers (projetos/diretórios indexados)
CREATE TABLE buffers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    path TEXT NOT NULL,
    total_chunks INTEGER DEFAULT 0,
    total_files INTEGER DEFAULT 0,
    embedding_model TEXT,
    embedding_dims INTEGER,
    last_indexed_at INTEGER,
    created_at INTEGER DEFAULT (unixepoch())
) STRICT;

-- Fila de tasks para dispatch/aggregate
CREATE TABLE tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    buffer_id INTEGER NOT NULL,
    chunk_id INTEGER,
    status TEXT DEFAULT 'pending',  -- 'pending', 'running', 'done', 'failed'
    assigned_to TEXT,               -- ID do subagente
    payload JSONB,                  -- JSON binário compacto (v3.45+)
    result JSONB,                   -- JSON binário compacto (v3.45+)
    created_at INTEGER DEFAULT (unixepoch()),
    started_at INTEGER,
    finished_at INTEGER,
    FOREIGN KEY (chunk_id) REFERENCES chunks(id)
) STRICT;

-- Resultados de subagentes
CREATE TABLE findings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL,
    chunk_id INTEGER,
    finding_type TEXT,              -- 'bug', 'pattern', 'insight'
    content TEXT NOT NULL,
    confidence REAL,
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (task_id) REFERENCES tasks(id),
    FOREIGN KEY (chunk_id) REFERENCES chunks(id)
) STRICT;

-- Histórico de consultas
CREATE TABLE history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    buffer_id INTEGER,
    query TEXT NOT NULL,
    query_type TEXT,                -- 'search', 'context', 'query', 'run'
    results_count INTEGER,
    duration_ms INTEGER,
    used_by TEXT,                   -- 'opencode', 'pi', 'cursor'
    result_hash BLOB,
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (buffer_id) REFERENCES buffers(id)
) STRICT;

-- Padrões extraídos de análises
CREATE TABLE patterns (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    buffer_id INTEGER,
    pattern_type TEXT,              -- 'architectural', 'convention', 'anti-pattern'
    name TEXT NOT NULL,
    description TEXT,
    examples JSONB,                 -- JSON array de chunk_ids (v3.45+)
    confidence REAL,
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (buffer_id) REFERENCES buffers(id)
) STRICT;

-- Índices
-- idx_chunks_buffer REMOVIDO: é prefixo de idx_chunks_buffer_hash/file (redundante)
CREATE INDEX idx_chunks_file ON chunks(file_path);
CREATE INDEX idx_chunks_hash ON chunks(hash);
CREATE INDEX idx_chunks_buffer_hash ON chunks(buffer_id, hash);   -- dedup incremental
CREATE INDEX idx_chunks_buffer_file ON chunks(buffer_id, file_path);
CREATE INDEX idx_tasks_buffer ON tasks(buffer_id);
CREATE INDEX idx_tasks_status ON tasks(status);
CREATE INDEX idx_tasks_pending ON tasks(buffer_id) WHERE status = 'pending';  -- fila dispatch
CREATE INDEX idx_findings_task ON findings(task_id);
CREATE INDEX idx_findings_chunk ON findings(chunk_id);
CREATE INDEX idx_history_buffer ON history(buffer_id);
CREATE INDEX idx_history_buffer_created ON history(buffer_id, created_at DESC);
CREATE INDEX idx_patterns_buffer ON patterns(buffer_id);

-- Após criar o schema e após grandes loads:
-- ANALYZE;
```

### Tabelas de Operação (Planos 12-14)

```sql
-- ════════ PLANO 12: Budget e Custo ════════

-- Runs com custo agregado (accountability por agente)
CREATE TABLE runs (
    id TEXT PRIMARY KEY,
    task TEXT NOT NULL,
    backend TEXT,
    mode TEXT,
    status TEXT,
    agent TEXT,                  -- 'opencode', 'pi', 'cursor', 'cli'
    started_at INTEGER,
    finished_at INTEGER,
    duration_ms INTEGER,
    total_cost REAL DEFAULT 0,
    total_tokens INTEGER DEFAULT 0,
    total_calls INTEGER DEFAULT 0,
    max_depth INTEGER,
    nodes_visited INTEGER,
    partial_answer TEXT,
    error TEXT
) STRICT WITHOUT ROWID;   -- PK TEXT: 1 B-tree (cluster) em vez de 2

-- Uso por modelo dentro de uma run
CREATE TABLE run_model_usage (
    run_id TEXT NOT NULL,
    model TEXT NOT NULL,
    calls INTEGER DEFAULT 0,
    input_tokens INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    cost REAL DEFAULT 0,
    PRIMARY KEY (run_id, model),
    FOREIGN KEY (run_id) REFERENCES runs(id)
) STRICT WITHOUT ROWID;

-- Custo por nó (análise granular)
CREATE TABLE node_calls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    depth INTEGER,
    node_type TEXT,              -- 'planner' | 'solver' | 'synthesizer'
    model TEXT,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cost REAL,
    duration_ms INTEGER,
    status TEXT,                 -- 'ok' | 'error' | 'retried' | 'cached'
    error TEXT,
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (run_id) REFERENCES runs(id)
) STRICT;

CREATE VIEW agent_cost_report AS
SELECT
    agent,
    COUNT(*) as runs,
    SUM(total_cost) as total_cost,
    SUM(total_tokens) as total_tokens,
    AVG(duration_ms) as avg_duration_ms
FROM runs
GROUP BY agent;

-- ════════ PLANO 13: Trajectórias e Sessões ════════

-- Trajectória completa (aprendizado)
CREATE TABLE trajectories (
    id TEXT PRIMARY KEY,          -- run_id
    project TEXT NOT NULL,
    agent TEXT,
    trajectory_json JSONB NOT NULL,-- TrajectoryNode serializado
    task TEXT NOT NULL,
    result_hash TEXT,             -- hash da resposta final (dedup)
    created_at INTEGER DEFAULT (unixepoch())
) STRICT WITHOUT ROWID;

-- Sessões multi-turn
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    title TEXT,
    created_at INTEGER,
    updated_at INTEGER,
    context_count INTEGER DEFAULT 0,
    history_count INTEGER DEFAULT 0
) STRICT WITHOUT ROWID;

-- Contextos versionados → context_0, context_1...
CREATE TABLE session_contexts (
    session_id TEXT NOT NULL,
    context_index INTEGER NOT NULL,
    payload JSONB NOT NULL,
    created_at INTEGER,
    PRIMARY KEY (session_id, context_index),
    FOREIGN KEY (session_id) REFERENCES sessions(id)
) STRICT WITHOUT ROWID;

-- Históricos versionados → history_0, history_1...
CREATE TABLE session_histories (
    session_id TEXT NOT NULL,
    history_index INTEGER NOT NULL,
    messages_json JSONB NOT NULL, -- deep copy serializado
    created_at INTEGER,
    PRIMARY KEY (session_id, history_index),
    FOREIGN KEY (session_id) REFERENCES sessions(id)
) STRICT WITHOUT ROWID;

-- ════════ PLANO 14: Caching e Observabilidade ════════

-- Cache de resultados (dedup de subtasks)
CREATE TABLE result_cache (
    task_hash TEXT NOT NULL,
    project TEXT NOT NULL,
    result TEXT NOT NULL,
    run_id TEXT,
    created_at INTEGER DEFAULT (unixepoch()),
    hit_count INTEGER DEFAULT 1,
    PRIMARY KEY (task_hash, project),
    FOREIGN KEY (run_id) REFERENCES runs(id)
) STRICT WITHOUT ROWID;

-- Eventos persistidos (replay/auditoria)
CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    event_json JSONB NOT NULL,    -- RlmEvent serializado
    created_at INTEGER DEFAULT (unixepoch())
) STRICT;

-- Índices das tabelas de operação
CREATE INDEX idx_runs_agent ON runs(agent);
CREATE INDEX idx_runs_status ON runs(status);
CREATE INDEX idx_runs_started ON runs(started_at);
CREATE INDEX idx_trajectories_project ON trajectories(project);
CREATE INDEX idx_trajectories_task ON trajectories(task);
CREATE INDEX idx_trajectories_result_hash ON trajectories(result_hash);
CREATE INDEX idx_events_run ON events(run_id);
CREATE INDEX idx_events_created ON events(created_at);
CREATE INDEX idx_node_calls_run ON node_calls(run_id);   -- relatórios de custo (plano 12)
-- idx_result_cache_hash REMOVIDO: PK (task_hash, project) já cobre task_hash
```

### FTS5 (Busca Textual)

```sql
-- ⚠️ O conteúdo de chunk_texts é comprimido (zstd) e NÃO pode ser tokenizado.
-- FTS5 é CONTENTLESS: o índice recebe o TEXTO PURO via pipeline de ingestão
-- (automerge controlado, sem triggers por linha → ingestão mais rápida).

CREATE VIRTUAL TABLE chunks_fts USING fts5(
    content,
    buffer_id UNINDEXED,            -- filtro por buffer sem JOIN extra
    content='',
    tokenize='unicode61',           -- porter funde identificadores de código
    prefix='2,3',                   -- prefix queries em identificadores: O(log N)
    detail='none'                   -- sem posições: -50% índice, +velocidade (sem frases/NEAR)
);

-- Ingestão em massa (feito pelo pipeline, texto puro):
--   INSERT INTO chunks_fts(chunks_fts) VALUES('automerge=0');
--   INSERT INTO chunks_fts(rowid, content, buffer_id) VALUES (?, ?, ?);
--   INSERT INTO chunks_fts(chunks_fts) VALUES('automerge=2');
--   INSERT INTO chunks_fts(chunks_fts) VALUES('merge=1000');
--   INSERT INTO chunks_fts(chunks_fts) VALUES('optimize');

-- Delete (contentless exige rowid + conteúdo original):
--   INSERT INTO chunks_fts(chunks_fts, rowid, content, buffer_id)
--     VALUES('delete', ?, <texto original descomprimido>, ?);

-- Busca BM25 com filtro de buffer:
--   SELECT rowid, bm25(chunks_fts) AS rank
--   FROM chunks_fts
--   WHERE chunks_fts MATCH ? AND buffer_id = ?
--   ORDER BY rank LIMIT ?;
```

### Otimizações SQLite

```sql
-- Aplicados ao abrir conexão. ⚠️ ORDEM IMPORTA:
PRAGMA page_size=8192;            -- ANTES de qualquer write (não muda depois; WAL bloqueia troca)
PRAGMA journal_mode=WAL;          -- Melhor concorrência
PRAGMA synchronous=NORMAL;        -- Seguro com WAL, ganho enorme de escrita
PRAGMA mmap_size=268435456;       -- 256MB de cache mapeado
PRAGMA cache_size=-65536;         -- 64MB de cache
PRAGMA temp_store=MEMORY;
PRAGMA busy_timeout=5000;         -- 5s de espera em lock
PRAGMA wal_autocheckpoint=2000;   -- checkpoint a cada ~16MB (páginas de 8KB)
PRAGMA journal_size_limit=33554432; -- cap de 32MB para o WAL
PRAGMA hard_heap_limit=104857600; -- 100MB hard limit (embarcado: evita OOM)
PRAGMA optimize;                  -- aplica estatísticas na abertura

-- Bulk ingest (dentro da transação grande):
--   PRAGMA synchronous=OFF;            -- só durante bulk (WAL + dados reindexáveis): 2-10x
--   PRAGMA wal_autocheckpoint=0;       -- evita checkpoint no meio do load
--   ... inserts ...
--   PRAGMA wal_checkpoint(FULL);       -- consolida WAL no banco
--   PRAGMA wal_autocheckpoint=2000;
--   PRAGMA synchronous=NORMAL;

-- Deploy single-process (CLI local): elimina o arquivo -shm (wal-index em heap)
--   PRAGMA locking_mode=EXCLUSIVE;
```

### Migrações

```sql
-- version 1
CREATE TABLE schema_version (
    version INTEGER PRIMARY KEY,
    applied_at INTEGER DEFAULT (unixepoch())
);

-- Migrações são scripts SQL numerados
-- migrations/001_initial.sql            ← schema base (chunks, buffers, tasks...)
-- migrations/002_add_patterns.sql       ← patterns table
-- migrations/003_add_watch_state.sql    ← watch state
-- migrations/004_add_runs_cost.sql      ← plan 12: runs, run_model_usage, node_calls
-- migrations/005_add_trajectories.sql   ← plan 13: trajectories
-- migrations/006_add_sessions.sql       ← plan 13: sessions, session_contexts, session_histories
-- migrations/007_add_result_cache.sql   ← plan 14: result_cache
-- migrations/008_add_events.sql         ← plan 14: events
```

## Compilação do SQLite (bundled)

O `rusqlite` com feature `bundled` compila o SQLite a partir do source. Flags de
compilação: ~5% de CPU, checkpoint WAL muito mais barato no Linux e leitura de
BLOB direto do disco. Aplicar via variável de ambiente `SQLITE3_FLAGS`
(libsqlite3-sys repassa `-D` ao compilador do SQLite):

```bash
# Cargo.toml
rusqlite = { version = "0.32", features = ["bundled"] }

# build: flags do SQLite
export SQLITE3_FLAGS="
    -DSQLITE_DQS=0
    -DSQLITE_DEFAULT_MEMSTATUS=0
    -DSQLITE_DEFAULT_WAL_SYNCHRONOUS=1
    -DSQLITE_DIRECT_OVERFLOW_READ
    -DSQLITE_ENABLE_BATCH_ATOMIC_WRITE
    -DSQLITE_OMIT_SHARED_CACHE
    -DSQLITE_USE_ALLOCA
    -DSQLITE_HAVE_MALLOC_USABLE_SIZE
    -DSQLITE_BYTEORDER=1234
"
cargo build --release
```

| Flag | Efeito |
|------|--------|
| `SQLITE_DIRECT_OVERFLOW_READ` | lê páginas overflow (BLOB zstd) direto do disco |
| `SQLITE_ENABLE_BATCH_ATOMIC_WRITE` | checkpoint WAL atômico no Linux (grande ganho) |
| `SQLITE_DEFAULT_WAL_SYNCHRONOUS=1` | synchronous=NORMAL como padrão no WAL |
| `SQLITE_DEFAULT_MEMSTATUS=0` | acelera sqlite3_malloc |
| `SQLITE_OMIT_SHARED_CACHE` | remove overhead do cache compartilhado |
| `SQLITE_USE_ALLOCA` / `SQLITE_HAVE_MALLOC_USABLE_SIZE` | alocações mais rápidas |

⚠️ `SQLITE_THREADSAFE=0` só se o SQLite for acessado por UMA thread (CLI pura).
No modo `serve`/subagentes, manter serializado (default).

## usearch Schema

### Tabela de Vetores

```rust
// Schema do usearch
let schema = Arc::new(Schema::new(vec![
    Field::new("chunk_id", DataType::UInt64, false),
    Field::new("buffer_id", DataType::UInt64, false),
    Field::new("vector", DataType::FixedSizeList(
        Arc::new(Field::new("item", DataType::Float32, true)),
        1024,  // BGE-M3 dims
    ), false),
]));

// Índice HNSW
table.create_index(
    Index::hnsw(VectorIndexParams {
        metric: MetricType::L2,
        m: 16,
        ef_construction: 200,
        ef_search: 100,
    })
).await?;
```

### Parâmetros HNSW

| Parâmetro | Valor | Descrição |
|-----------|-------|-----------|
| `m` | 16 | Conexões por nó (recall vs内存) |
| `ef_construction` | 200 | Tamanho da lista durante construção |
| `ef_search` | 100 | Tamanho da lista durante busca |
| `metric` | L2 | Distância euclidiana |

## Connection Pool

```rust
pub struct Storage {
    sqlite: Arc<Mutex<rusqlite::Connection>>,  // Connection é Send, não Sync → Mutex
    lance: Arc<usearch::Connection>,
    path: PathBuf,
}

impl Storage {
    pub fn open(path: &Path) -> Result<Self> {
        let sqlite_path = path.join("knowledge.db");
        let lance_path = path.join("vectors.lance");

        // SQLite com WAL — ORDEM IMPORTANTE: page_size ANTES de qualquer write
        let sqlite = rusqlite::Connection::open(&sqlite_path)?;
        sqlite.execute_batch("
            PRAGMA page_size=8192;
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            PRAGMA mmap_size=268435456;
            PRAGMA cache_size=-65536;
            PRAGMA temp_store=MEMORY;
            PRAGMA busy_timeout=5000;
            PRAGMA wal_autocheckpoint=2000;
            PRAGMA journal_size_limit=33554432;
            PRAGMA hard_heap_limit=104857600;
            PRAGMA optimize;
        ")?;
        // No deploy single-process (CLI), também:
        //   PRAGMA locking_mode=EXCLUSIVE;  (elimina -shm)

        // usearch
        let lance = usearch::connect(&lance_path.to_string_lossy())
            .execute()
            .await?;

        Ok(Self {
            sqlite: Arc::new(Mutex::new(sqlite)),
            lance: Arc::new(lance),
            path: path.to_path_buf(),
        })
    }

    /// Transação dual (SQLite + usearch)
    pub fn transaction<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Transaction) -> Result<R>,
    {
        let mut guard = self.sqlite.lock().unwrap();
        // IMMEDIATE evita SQLITE_BUSY no upgrade DEFERRED→RESERVED sob contenção
        let tx = guard.transaction_with_behavior(
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let lance_tx = self.lance.begin_transaction().await?;

        let result = f(&Transaction {
            sqlite: &tx,
            lance: &lance_tx,
        })?;

        tx.commit()?;
        lance_tx.commit().await?;

        Ok(result)
    }
}
```

## Compressão de Texto

```rust
use zstd::stream::{Encoder, Decoder};

pub fn compress_text(text: &str) -> Vec<u8> {
    let mut encoder = Encoder::new(Vec::new(), 3).unwrap();
    encoder.write_all(text.as_bytes()).unwrap();
    encoder.finish().unwrap()
}

pub fn decompress_text(data: &[u8]) -> Result<String> {
    let mut decoder = Decoder::new(data)?;
    let mut text = String::new();
    decoder.read_to_string(&mut text)?;
    Ok(text)
}
```

**Ganho típico:** Texto de código comprime ~60-70% com zstd nível 3.

## Manutenção Periódica

- `ANALYZE` após migrações e grandes loads (estatísticas do planejador; sem ele
  não há skip-scan nem bons planos de execução).
- `PRAGMA optimize` ao fechar conexões de curta duração.
- Após `index_incremental` (deletes de arquivos modificados) a freelist cresce:
  `VACUUM` periódico ou `PRAGMA auto_vacuum=INCREMENTAL` + `PRAGMA incremental_vacuum(100)`.
- `PRAGMA quick_check` periodicamente (rápido, ~1s por GB).

## Backup e Restore

```rust
impl Storage {
    pub fn backup(&self, dest: &Path) -> Result<()> {
        // SQLite backup
        self.sqlite.backup(
            &mut rusqlite::Connection::open(dest.join("knowledge.db"))?,
            "main",
            None,
        )?;

        // usearch backup (copia diretório)
        fs_extra::dir::copy(
            self.path.join("vectors.lance"),
            dest,
            &fs_extra::dir::CopyOptions::new(),
        )?;

        Ok(())
    }

    pub fn verify(&self) -> Result<VerifyResult> {
        // Verifica integridade do SQLite
        self.sqlite.execute_batch("PRAGMA integrity_check")?;

        // Verifica schema version
        let version: i32 = self.sqlite.query_row(
            "SELECT version FROM schema_version ORDER BY version DESC LIMIT 1",
            [],
            |row| row.get(0),
        )?;

        Ok(VerifyResult { version, ok: true })
    }
}
```
