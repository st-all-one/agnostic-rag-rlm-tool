# arlm-storage

Componente de persistência do arlm — SQLite (metadados, FTS5/BM25) + LanceDB (vetores HNSW).

## Responsabilidades

- **SQLite**: Schema com tabelas `chunks`, `buffers`, `tasks`, `findings`, `history`, `patterns`
- **FTS5**: Índice de busca textual (BM25) com tokenizer `porter unicode61`
- **LanceDB**: Armazenamento de vetores de embedding com índice HNSW
- **Otimizações SQLite**: WAL, mmap 256MB, cache 64MB, busy_timeout 5s

## Estrutura

```
src/
├── lib.rs              # Re-exports públicos
├── sqlite/
│   ├── conn.rs         # Storage::open(), connection management
│   ├── schema.rs       # Migrações SQL
│   ├── chunks.rs       # CRUD de chunks
│   ├── buffers.rs      # CRUD de buffers (projetos)
│   ├── tasks.rs        # Fila de tasks para dispatch
│   ├── findings.rs     # Resultados de subagentes
│   ├── history.rs      # Histórico de consultas
│   └── patterns.rs     # Padrões extraídos
├── lance/
│   ├── vectors.rs      # VectorStore::open(), insert, search
│   ├── index.rs        # Criação de índice HNSW
│   └── search.rs       # SearchResult, search_similar
migrations/
└── 001_initial.sql     # Schema SQL completo
```

## Uso

```rust
use arlm_storage::{Storage, VectorStore};

// Abrir storage SQLite
let storage = Storage::open(Path::new("~/.arlm/data"))?;

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

// Abrir vector store (async)
let vectors = VectorStore::open(Path::new("~/.arlm/data")).await?;

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
```

## Testes

```bash
cargo test -p arlm-storage
```

23 testes cobrindo: migrações, CRUD de chunks/buffers/tasks/findings/history/patterns, operações LanceDB.
