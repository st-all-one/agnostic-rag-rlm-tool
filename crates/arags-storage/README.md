# arags-storage

Componente de persistência do arags — SQLite (metadados, FTS5/BM25) + usearch (vetores HNSW, single-file).

## Responsabilidades

- **SQLite**: Schema com tabelas `chunks`, `buffers`, `tasks`, `findings`, `history`, `patterns`
- **FTS5**: Índice de busca textual (BM25) com tokenizer `porter unicode61`
- **usearch**: Armazenamento de vetores de embedding com índice HNSW single-file (substitui o LanceDB)
- **Backups**: `Storage::backup()` (`VACUUM INTO`) e `Storage::verify()` (`PRAGMA integrity_check`)
- **RLM summaries (plan 021)**: `sqlite/rlm/` — nós/jobs/edges com review gate,
  conclusão transacional e staleness por hash
- **QA-Cache (plan 017)**: `qa_cache.rs` — tabela `qa_cache` + FTS5 + índice vetorial
  dedicado (`question_vectors`, usearch) para respostas digeridas pelo client;
  `store_answer` idempotente (reserve-lock), lookup por `(project, question_hash)`,
  staleness por hash de chunk e eviction LRU ponderado.
- **Explorations (plan 022/023)**: `sqlite/explorations/` — mapas relacionais
  com âncoras por hash, epochs, feedback com auto-retire e review gate
  (`pending_review` + `review_exploration`).
- **Auth tokens (plan 018)**: `tokens.rs` — `auth_tokens`/`auth_sessions` para
  refresh-token rotation + sessões de curta duração com roles `Admin`/`NonAdmin`.
- **Single DB**: Todos os projetos compartilham `~/.arags/knowledge.db`
- **UUIDv7**: Cada buffer (projeto) tem UUIDv7 único

## Estrutura

```
src/
├── lib.rs              # Re-exports públicos
├── sqlite/
│   ├── conn.rs         # Storage::open()/open_pooled(), pragmas, backup/verify
│   ├── schema.rs       # 20 migrações SQL versionadas
│   ├── chunks.rs       # CRUD de chunks + trust helpers (hashes_match/ages_hours)
│   ├── buffers.rs      # CRUD de buffers com UUIDv7
│   ├── tasks.rs        # Fila de tasks para dispatch
│   ├── findings.rs     # Resultados de subagentes
│   ├── history/        # Histórico de consultas (retenção server-side)
│   ├── patterns.rs     # Padrões extraídos
│   ├── entities.rs     # Busca por entidades
│   ├── rlm/            # RLM summaries (nodes/jobs/complete/graph; plan 021)
│   ├── tokens/         # Auth (plan 018): refresh rotation + sessões
│   ├── qa_cache.rs     # QA-Cache (plan 017): lookup/staleness/eviction
│   └── explorations/   # Explorations (plan 022): store/staleness/feedback + review gate
├── vector_space.rs     # VectorSpaceStore genérico (plan 023): debounced save
├── lance/
│   └── vectors.rs      # VectorStore::open(), insert, search (usearch), SearchResult
├── qa_vectors.rs       # Espaço B: perguntas QA (facade do VectorSpaceStore)
├── rlm_vectors.rs      # Espaço C: sumários RLM (facade)
└── exploration_vectors.rs  # Espaço D: mapas de exploração (facade)
migrations/
├── 001_initial.sql
├── ...
├── 018_add_rlm.sql
├── 019_add_explorations.sql
└── 020_add_exploration_review.sql
```

## Arquitetura de Dados

```
~/.arags/
├── knowledge.db                  # SQLite (WAL, FTS5, metadados)
├── knowledge.db-wal              # WAL journal
├── vectors.usearch (+ .meta)     # chunks (HNSW L2, 384-dim) + buffer map
├── question_vectors.usearch      # espaço B: perguntas QA (cosseno)
├── rlm_vectors.usearch           # espaço C: sumários RLM (cosseno)
└── exploration_vectors.usearch   # espaço D: mapas de exploração (cosseno)
```

### Single Database

Todos os projetos compartilham um único DB. Isolamento por `buffer_id` em todas as tabelas:

```sql
-- Cada projeto é um buffer
INSERT INTO buffers (name, path, uuid) VALUES ('projeto-a', '/path', '...');

-- Chunks linkam ao buffer
INSERT INTO chunks (buffer_id, file_path, ...) VALUES (1, 'src/main.rs', ...);
```

### UUIDv7

```rust
// UUIDv7 é gerado automaticamente no insert
let buffer_id = storage.insert_buffer(&NewBuffer {
    name: "meu-projeto".to_string(),
    path: "/path/to/project".to_string(),
})?;

// Buscar por UUID
let buffer = storage.get_buffer_by_uuid("01912345-...")?;

// Backfill de UUIDs em buffers existentes
storage.ensure_uuids()?;
```

## Uso

```rust
use arags_storage::{Storage, VectorStore};
use arags_storage::sqlite::buffers::NewBuffer;

// Abrir storage (single DB compartilhado)
let storage = Storage::open(Path::new("~/.arags"))?;

// Inserir buffer (projeto)
let buffer_id = storage.insert_buffer(&NewBuffer {
    name: "meu-projeto".to_string(),
    path: "/path/to/project".to_string(),
})?;

// Inserir chunk
let chunk_id = storage.insert_chunk(&NewChunk {
    buffer_id,
    file_path: "src/main.rs".to_string(),
    offset_start: 0,
    offset_end: 100,
    line_start: 1,
    line_end: 10,
    hash: vec![0x01, 0x02],
    language: Some("rust".to_string()),
    chunk_type: Some("function".to_string()),
    token_count: Some(50),
})?;

// Listar todos os projetos
let buffers = storage.list_buffers()?;

// Abrir vector store (async)
let vectors = VectorStore::open(Path::new("~/.arags")).await?;

// Inserir vetores
vectors.insert_vectors(&[VectorEntry {
    chunk_id: 1,
    buffer_id: 1,
    vector: vec![0.1; 384], // all-MiniLM-L6-v2 dims
}]).await?;
```

## SQLite Pragmas

```sql
PRAGMA page_size=8192;
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA mmap_size=268435456;       -- 256MB
PRAGMA cache_size=-65536;         -- 64MB
PRAGMA temp_store=MEMORY;
PRAGMA busy_timeout=5000;
PRAGMA wal_autocheckpoint=2000;
PRAGMA journal_size_limit=33554432; -- 32MB
PRAGMA hard_heap_limit=104857600;   -- 100MB
PRAGMA threads=4;                   -- sort paralelo
PRAGMA automatic_index=ON;          -- índices automáticos
PRAGMA analysis_limit=1000;         -- ANALYZE rápido
PRAGMA locking_mode=EXCLUSIVE;      -- CLI only (open_exclusive)
```

## Migrações

Schema versionado com 20 migrações (ver `migrations/`):
- `001_initial` — Schema base (chunks, buffers, tasks, findings, history, patterns)
- `004`–`013` — Runs/custos, trajectories, caches, eventos, entidades, UUIDv7, server handlers
- `015` — Auth (plan 018): `auth_tokens` + `auth_sessions` (refresh/sessões)
- `016` — QA-Cache (plan 017): `qa_cache` + `qa_cache_fts` + triggers
- `018` — RLM (plan 021): `rlm_nodes`/`rlm_jobs`/`rlm_edges` + FTS
- `019` — Explorations (plan 022): mapas, âncoras, FTS, `project_epochs`
- `020` — Review gate (plan 023): status `'pending_review'` em `explorations`

## Espaços vetoriais dedicados (plan 023)

Os três índices secundários compartilham o núcleo genérico
[`VectorSpaceStore`](src/vector_space.rs) (usearch cosseno, chave = rowid da
tabela correspondente) com **persistência debounced** (2s por espaço, flag
dirty) e flush uniforme via trait `FlushableVectorSpace` no graceful shutdown
do servidor. Rajadas de inserts (bulk answers, conclusões RLM) amortizam para
um único write O(N) do arquivo.

O espaço A (chunks) também expõe `VectorStore::delete_chunk_ids` (async) e
`delete_chunk_ids_blocking` (síncrono) para purgar vetores de chunks removidos
(em consolidação/decay) — mantém o índice usearch em sincronia com o SQLite e
evita o rebuild completo no bootstrap (`agnostic-rlm-rs-fa25`).

## Uso Exclusive (CLI)

```rust
// Para CLI single-process (elimina arquivo -shm)
let storage = Storage::open_exclusive(Path::new("~/.arags"))?;
```

## Uso Pooled Híbrido (server, plan 020)

```rust
// pool_size > 1 no server.toml: escritas concorrentes via pool (connection()),
// leituras na conexão compartilhada dedicada (conn()) — válido em ambos os modos.
let storage = Storage::open_pooled(Path::new("/data/arags"), 4)?;

// Flusher de WAL do server (`flush_interval_ms`):
storage.wal_checkpoint()?;

// Retenção de histórico (`[history] retention_days`):
let removed = storage.purge_history_before(cutoff_unix)?;
```

## Testes

```bash
cargo test -p arags-storage
```

Testes cobrindo: migrações, CRUD de chunks/buffers/tasks/findings/history/patterns/summaries,
UUIDv7, backup/verify, FTS5, vector store (usearch) com buffer filter e persistência,
`qa_cache` (hit/stale/eviction/scoping/reserve-lock) e auth tokens/sessões (plan 018).
