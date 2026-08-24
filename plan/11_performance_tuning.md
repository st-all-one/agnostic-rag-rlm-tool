# Performance Tuning — Detalhes de Otimização

## Visão Geral

O `arags` é otimizado para três cenários:
1. **Ingestão:** Processar milhares de arquivos o mais rápido possível
2. **Busca:** Retornar resultados em <100ms
3. **RLM:** Executar recursão com latência mínima entre nós

## Benchmarks Alvo

| Operação | Target | Tolerável | Crítico |
|----------|--------|-----------|---------|
| Ingestão (100MB) | <30s | <60s | >120s |
| Busca híbrida | <30ms | <100ms | >500ms |
| Embedding (batch 64) | <1s | <3s | >10s |
| RLM node (planner) | <2s | <5s | >15s |
| RLM run (depth 3) | <30s | <60s | >120s |
| Memória (peak) | <500MB | <1GB | >2GB |

## Otimização de CPU

### Rayon (Paralelismo de Dados)

```rust
use rayon::prelude::*;

// Configuração do pool de threads
pub fn configure_rayon() {
    rayon::ThreadPoolBuilder::new()
        .num_threads(num_cpus::get_physical())  // Cores físicos, não lógicos
        .thread_name(|idx| format!("arags-worker-{}", idx))
        .build_global()
        .unwrap();
}

// Chunking paralelo
pub fn chunk_files_parallel(files: &[PathBuf]) -> Vec<RawChunk> {
    files.par_iter()  // par_iter() em vez de iter()
        .flat_map(|file| {
            let mmap = read_file_mmap(file).unwrap();
            let content = std::str::from_utf8(&mmap).unwrap();
            let strategy = get_strategy(file);
            strategy.chunk(content, file)
        })
        .collect()
}

// Embedding em lote paralelo
pub fn embed_batch_parallel(
    texts: &[String],
    embedder: &BgeM3Embedder,
    batch_size: usize,
) -> Vec<Vec<f32>> {
    texts.par_chunks(batch_size)
        .flat_map(|batch| {
            embedder.embed_batch(batch).unwrap()
        })
        .collect()
}
```

### CPU Features (Compile-time)

```toml
# Cargo.toml
[target.'cfg(target_arch = "x86_64")']
rustflags = ["-C", "target-cpu=native"]

[target.'cfg(target_arch = "aarch64")']
rustflags = ["-C", "target-cpu=apple-m1"]
```

**Ganho típico:** 20-40% em operações SIMD (embedding, hashing)

## Otimização de I/O

### Memmap2 (Zero-Copy)

```rust
use memmap2::Mmap;

// LEITURA: zero-copy, delega ao OS
pub fn read_file_mmap(path: &Path) -> Result<Mmap> {
    let file = File::open(path)?;
    let mmap = unsafe { Mmap::map(&file)? };
    Ok(mmap)
}

// Para arquivo de 1GB:
// - Sem memmap: 1GB na RAM + cópia para buffer
// - Com memmap: 0 bytes na RAM, pages sob demanda
```

### SQLite WAL + mmap

```sql
-- Aplicados ao abrir conexão. ⚠️ ORDEM IMPORTA: page_size ANTES de qualquer write
-- (não muda depois; WAL bloqueia troca de page_size)
PRAGMA page_size=8192;            -- páginas maiores para BLOB zstd
PRAGMA journal_mode=WAL;          -- Writes não bloqueiam reads
PRAGMA synchronous=NORMAL;        -- Commit mais rápido (seguro com WAL)
PRAGMA mmap_size=268435456;       -- 256MB mapeado em memória
PRAGMA cache_size=-65536;         -- 64MB de cache
PRAGMA temp_store=MEMORY;         -- Temp tables em RAM
PRAGMA busy_timeout=5000;         -- Espera 5s em lock
PRAGMA wal_autocheckpoint=2000;   -- checkpoint a cada ~16MB
PRAGMA journal_size_limit=33554432; -- cap do WAL em 32MB
PRAGMA hard_heap_limit=104857600; -- 100MB hard limit (embarcado)

-- Bulk ingest (dentro da transação grande): 2-10x
PRAGMA synchronous=OFF;           -- só durante bulk (WAL + dados reindexáveis)
PRAGMA wal_autocheckpoint=0;      -- evita checkpoint no meio do load
-- ... inserts ...
PRAGMA wal_checkpoint(FULL);      -- consolida WAL no banco
PRAGMA wal_autocheckpoint=2000;
PRAGMA synchronous=NORMAL;
```

### Batch Inserts

```rust
// INSERÇÃO EM LOTE: 10x mais rápido que inserts individuais
pub fn insert_chunks_batch(
    conn: &Connection,
    chunks: &[ChunkWithText],
) -> Result<()> {
    let mut stmt = conn.prepare("
        INSERT INTO chunks (buffer_id, file_path, offset_start, offset_end, hash)
        VALUES (?1, ?2, ?3, ?4, ?5)
    ")?;

    let mut tx = conn.transaction_with_behavior(
        TransactionBehavior::Immediate,
    )?;

    for chunk in chunks {
        stmt.execute(params![
            chunk.buffer_id,
            chunk.file_path,
            chunk.offset_start,
            chunk.offset_end,
            chunk.hash,
        ])?;
    }

    tx.commit()?;
    Ok(())
}

// Bulk de verdade: usar multi-row INSERT + synchronous=OFF + wal_autocheckpoint=0
// dentro da transação (ver bloco "SQLite WAL + mmap" acima). Restaurar no fim.
//   INSERT INTO chunks (...) VALUES (?1,...),(?2,...),...
// multi-row reduz round-trips SQLite→C; em lote de 100+, ganho adicional.

// Via batch API (ainda mais rápido)
pub fn insert_chunks_direct(
    conn: &Connection,
    chunks: &[ChunkWithText],
) -> Result<()> {
    let mut writer = conn.writer()?;
    for chunk in chunks {
        writer.write_row(params![
            chunk.buffer_id,
            chunk.file_path,
            chunk.offset_start,
            chunk.offset_end,
            chunk.hash,
        ])?;
    }
    writer.flush()?;
    Ok(())
}
```

## Otimização de Embedding

### Batch Size Ótimo

```rust
// Benchmark: BGE-M3 INT8 no CPU
// Batch 1:   100 chunks/s
// Batch 16:  400 chunks/s  (+300%)
// Batch 32:  600 chunks/s  (+50%)
// Batch 64:  800 chunks/s  (+33%)
// Batch 128: 850 chunks/s  (+6%)  ← diminishing returns

const OPTIMAL_BATCH_SIZE: usize = 64;
```

### Quantização INT8

```rust
// Modelo BGE-M3 quantizado INT8
// - FP32: 2.4GB, 100 chunks/s
// - INT8: 600MB, 800 chunks/s  ← 8x mais rápido, 4x menor

let model = BgeM3Embedder::load_quantized(
    model_path,
    Quantization::Int8,
)?;
```

### Cache de Embeddings

```rust
// Cache em SQLite para evitar re-embedding
pub struct EmbeddingCache {
    conn: Connection,
}

impl EmbeddingCache {
    pub fn get_or_compute(
        &self,
        text: &str,
        embedder: &BgeM3Embedder,
    ) -> Result<Vec<f32>> {
        let hash = compute_hash(text);

        // Tenta cache
        if let Some(embedding) = self.get_by_hash(&hash)? {
            return Ok(embedding);
        }

        // Compute e cache
        let embedding = embedder.embed(text)?;
        self.insert(hash, &embedding)?;

        Ok(embedding)
    }
}
```

## Otimização de Busca

### Índices Otimizados

```sql
-- Índices compostos para queries frequentes
CREATE INDEX idx_chunks_buffer_file ON chunks(buffer_id, file_path);
CREATE INDEX idx_chunks_buffer_hash ON chunks(buffer_id, hash);
CREATE INDEX idx_chunks_status ON chunks(status);

-- Índice parcial para chunks ativos
CREATE INDEX idx_chunks_active ON chunks(buffer_id) WHERE status = 'active';

-- Índice parcial para fila de dispatch
CREATE INDEX idx_tasks_pending ON tasks(buffer_id) WHERE status = 'pending';

-- Índice para relatórios de custo (plano 12)
CREATE INDEX idx_node_calls_run ON node_calls(run_id);

-- FTS5: o índice é a própria tabela virtual (sem índice extra aqui)
```

### Otimização de Schema

- **STRICT** em todas as tabelas (tipos homogêneos) → armazenamento mais compacto.
- **WITHOUT ROWID** nas tabelas de PK composta/TEXT (`runs`, `run_model_usage`,
  `trajectories`, `sessions`, `session_contexts`, `session_histories`,
  `result_cache`) → 1 B-tree em vez de 2.
- **JSONB** (`payload`, `result`, `trajectory_json`, `messages_json`,
  `examples`, `event_json`) → JSON binário compacto, ~50% menor (v3.45+).

### FTS5 Otimizado

```sql
-- contentless: índice recebe TEXTO PURO (o conteúdo em chunk_texts é zstd)
CREATE VIRTUAL TABLE chunks_fts USING fts5(
    content,
    buffer_id UNINDEXED,
    content='',
    tokenize='unicode61',       -- porter funde identificadores de código
    prefix='2,3',               -- prefix queries em código: O(log N)
    detail='none'               -- sem posições: -50% índice, sem frases/NEAR
);

-- Ingestão: desabilitar merge incremental, consolidar no fim
INSERT INTO chunks_fts(chunks_fts) VALUES('automerge=0');
-- ... inserts com texto puro ...
INSERT INTO chunks_fts(chunks_fts) VALUES('automerge=2');
INSERT INTO chunks_fts(chunks_fts) VALUES('merge=1000');
INSERT INTO chunks_fts(chunks_fts) VALUES('optimize');
```

### ANÁLISE E MANUTENÇÃO

- `ANALYZE` após migrações e grandes loads (sem ele não há skip-scan nem planos bons).
- `PRAGMA optimize` ao fechar conexões de curta duração.
- `VACUUM` periódico após deletes do `index_incremental` (freelist cresce).

### Compilação do SQLite (bundled)

Flags via `SQLITE3_FLAGS` (rusqlite bundled): ~5% CPU + checkpoint WAL atômico
no Linux + leitura de BLOB direto do disco.

```bash
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
```

⚠️ `SQLITE_THREADSAFE=0` apenas para CLI single-thread; em `serve` manter serializado.

### Precomputation

```rust
// Precomputa estatísticas por buffer
pub fn precompute_stats(conn: &Connection, buffer_id: u64) -> Result<BufferStats> {
    let stats = conn.query_row("
        SELECT
            COUNT(*) as total_chunks,
            COUNT(DISTINCT file_path) as total_files,
            AVG(token_count) as avg_tokens,
            SUM(LENGTH(content)) as total_size
        FROM chunks
        WHERE buffer_id = ?1 AND status = 'active'
    ", params![buffer_id], |row| {
        Ok(BufferStats {
            total_chunks: row.get(0)?,
            total_files: row.get(1)?,
            avg_tokens: row.get(2)?,
            total_size: row.get(3)?,
        })
    })?;

    Ok(stats)
}
```

## Otimização de Memória

### Pool de Objetos

```rust
// Reutiliza buffers em vez de alocar novos
pub struct BufferPool {
    buffers: Vec<Vec<u8>>,
    max_size: usize,
}

impl BufferPool {
    pub fn acquire(&mut self, min_capacity: usize) -> Vec<u8> {
        self.buffers
            .pop()
            .filter(|b| b.capacity() >= min_capacity)
            .unwrap_or_else(|| Vec::with_capacity(min_capacity))
    }

    pub fn release(&mut self, mut buffer: Vec<u8>) {
        buffer.clear();
        if self.buffers.len() < self.max_size {
            self.buffers.push(buffer);
        }
    }
}
```

### Streaming

```rust
// Stream de chunks em vez de carregar tudo na memória
pub fn stream_chunks(
    path: &Path,
) -> impl Iterator<Item = RawChunk> {
    let file = File::open(path).unwrap();
    let mmap = unsafe { Mmap::map(&file).unwrap() };

    ChunkIterator::new(mmap)
        .map(|chunk| {
            // Chunk é processado e descartado
            // Não acumula na memória
            process_chunk(chunk)
        })
}
```

### Limites de Memória

```rust
pub struct MemoryLimiter {
    max_memory: usize,
    current: AtomicUsize,
}

impl MemoryLimiter {
    pub fn try_allocate(&self, bytes: usize) -> bool {
        let current = self.current.load(Ordering::Relaxed);
        if current + bytes > self.max_memory {
            return false;
        }
        self.current.fetch_add(bytes, Ordering::SeqCst);
        true
    }

    pub fn release(&self, bytes: usize) {
        self.current.fetch_sub(bytes, Ordering::SeqCst);
    }
}
```

## Otimização de Build

### Profile Release

```toml
[profile.release]
lto = true                    # 5-15% mais rápido, 10-20% menor binário
codegen-units = 1             # 5-10% mais rápido (compilação mais lenta)
panic = "abort"               # 2-5% mais rápido, 10% menor binário
strip = true                  # 10-20% menor binário
opt-level = 3                 # Máxima otimização

[profile.release.build-override]
opt-level = 3                 # Otimiza dependências também
```

### Compile-time Optimization

```bash
# Build otimizado
RUSTFLAGS="-C target-cpu=native" cargo build --release

# Com mold (linker mais rápido para dev)
cargo install mold
RUSTFLAGS="-C linker=clang -C link-arg=-fuse-ld=mold" cargo build

# Com sccache (cache de compilação)
cargo install sccache
export RUSTC_WRAPPER=sccache
cargo build --release
```

## profiling

```bash
# Perf profiling
cargo install perf
cargo build --release
perf record -g target/release/arags index ./meu-projeto
perf report

# Flamegraph
cargo install flamegraph
cargo build --release
flamegraph -- target/release/arags index ./meu-projeto

# Criterion benchmarks
cargo bench --package arags-embedding
cargo bench --package arags-search
```

## Resultados Esperados

### Ingestão (Projeto 100MB, ~10k arquivos)

| Fase | Tempo | Throughput |
|------|-------|------------|
| File discovery | 0.5s | 20k files/s |
| Memmap + chunking | 8s | 12 MB/s/core |
| Embedding (batch 64) | 15s | 800 chunks/s |
| SQLite insert | 3s | 10k inserts/s |
| usearch insert | 2s | 5k inserts/s |
| FTS5 update | 1s | 10k inserts/s |
| **Total** | **~30s** | **~3.3 MB/s** |

### Busca (10k chunks)

| Fase | Tempo |
|------|-------|
| Query embedding | 5ms |
| Semantic search (usearch) | 10ms |
| BM25 search (FTS5) | 5ms |
| RRF fusion | 1ms |
| Text recovery | 5ms |
| **Total** | **~26ms** |

### RLM Run (Depth 3, ~15 nodes)

| Fase | Tempo |
|------|-------|
| Planner (per node) | 1.5s |
| Solver (per node) | 2s |
| Synthesizer (per node) | 1.5s |
| Context retrieval | 30ms × 15 = 0.5s |
| **Total** | **~45s** |
