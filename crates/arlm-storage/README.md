# arlm-storage

Componente de persistência do arlm — SQLite (metadados, FTS5/BM25) + LanceDB (vetores HNSW).

## Responsabilidades

- **SQLite**: Schema com tabelas `chunks`, `buffers`, `tasks`, `findings`, `history`, `patterns`
- **FTS5**: Índice de busca textual (BM25) com tokenizer `porter unicode61`
- **LanceDB**: Armazenamento de vetores de embedding com índice HNSW
- **Single DB**: Todos os projetos compartilham `~/.arlm/knowledge.db`
- **UUIDv7**: Cada buffer (projeto) tem UUIDv7 único

## Estrutura

```
src/
├── lib.rs              # Re-exports públicos
├── sqlite/
│   ├── conn.rs         # Storage::open(), connection management
│   ├── schema.rs       # 11 migrações SQL
│   ├── chunks.rs       # CRUD de chunks
│   ├── buffers.rs      # CRUD de buffers com UUIDv7
│   ├── tasks.rs        # Fila de tasks para dispatch
│   ├── findings.rs     # Resultados de subagentes
│   ├── history.rs      # Histórico de consultas
│   ├── patterns.rs     # Padrões extraídos
│   ├── entities.rs     # Busca por entidades
│   └── sessions.rs     # Sessões multi-turn
├── lance/
│   ├── vectors.rs      # VectorStore::open(), insert, search
│   ├── index.rs        # Criação de índice HNSW
│   └── search.rs       # SearchResult, search_similar
migrations/
├── 001_initial.sql
├── ...
└── 011_add_uuid_to_buffers.sql
```

## Arquitetura de Dados

```
~/.arlm/
├── knowledge.db          # SQLite (WAL, FTS5, metadados)
├── knowledge.db-wal      # WAL journal
└── vectors.lance/        # LanceDB (HNSW vetorial, 1024-dim)
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
use arlm_storage::{Storage, VectorStore};
use arlm_storage::sqlite::buffers::NewBuffer;

// Abrir storage (single DB compartilhado)
let storage = Storage::open(Path::new("~/.arlm"))?;

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
let vectors = VectorStore::open(Path::new("~/.arlm")).await?;

// Inserir vetores
vectors.insert_vectors(&[VectorEntry {
    chunk_id: 1,
    buffer_id: 1,
    vector: vec![0.1; 1024], // BGE-M3 dims
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

Schema versionado com 11 migrações:
- `001_initial` — Schema base (chunks, buffers, tasks, findings, history, patterns)
- `004` — Runs e custos
- `005` — Trajectories
- `006` — Sessões
- `007` — Result cache
- `008` — Eventos
- `009` — Entidades
- `010` — last_accessed_at
- `011` — UUIDv7 em buffers

## Uso Exclusive (CLI)

```rust
// Para CLI single-process (elimina arquivo -shm)
let storage = Storage::open_exclusive(Path::new("~/.arlm"))?;
```

## Testes

```bash
cargo test -p arlm-storage
```

29 testes cobrindo: migrações, CRUD de chunks/buffers/tasks/findings/history/patterns, UUIDv7, operações LanceDB.
