# Plan 016: Server-First Architecture

## Context

The current architecture is CLI-first: every command (run, search, index, query) opens its own SQLite connection, does work, and exits. The `arags serve` HTTP server is a secondary feature that re-opens Storage on every request.

This plan flips the model: the **server is the primary process** (long-running, always-on), and the **CLI becomes a thin gRPC client** that communicates with it. This enables:

- **Team-oriented operation**: multiple users share one server with one SQLite database
- **Responsiveness**: persistent connections, no per-request open/migrate overhead
- **Write queue**: batched writes to avoid SQLite single-writer bottleneck
- **Automatic summarization**: dual-layer data model (raw + summary) with auto-generation post-indexing
- **REPL removal**: the REPL paradigm doesn't fit a server model; summarization replaces it

---

## Architecture Overview

```
┌─────────────────────────────────────────────────┐
│                   arags-server                    │
│            (long-running, always-on)             │
│                                                  │
│  ┌──────────┐  ┌──────────┐  ┌───────────────┐ │
│  │ SQLite   │  │ usearch  │  │ Summarization │ │
│  │ (r2d2    │  │ (vectors)│  │ Engine        │ │
│  │  pool)   │  │          │  │ (background)  │ │
│  └────┬─────┘  └────┬─────┘  └───────┬───────┘ │
│       │              │                │          │
│  ┌────┴──────────────┴────────────────┴───────┐ │
│  │           Write Queue (batched)            │ │
│  └────────────────────┬───────────────────────┘ │
│                       │                          │
│  ┌────────────────────┴───────────────────────┐ │
│  │         gRPC API (tonic + protobuf)        │ │
│  └────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────┘
                         │
                    gRPC (protobuf)
                         │
┌────────────────────────┴───────────────────────┐
│                   arags-cli                      │
│              (thin gRPC client)                 │
│                                                  │
│  ┌────────────┐  ┌────────────┐  ┌───────────┐ │
│  │ index      │  │ search     │  │ run       │ │
│  │ context    │  │ query      │  │ status    │ │
│  │ session    │  │ history    │  │ serve*    │ │
│  └────────────┘  └────────────┘  └───────────┘ │
│                                                  │
│  * serve = start the server (becomes primary)    │
└─────────────────────────────────────────────────┘
```

---

## New Crate: `arags-proto`

**Responsibility:** Protobuf definitions and generated Rust types for client-server communication.

### Proto files

```protobuf
syntax = "proto3";
package arags;

// ── Project Management ──

message CreateProjectRequest {
  string name = 1;
  string root_path = 2;
}

message ProjectInfo {
  string id = 1;
  string name = 2;
  string root_path = 3;
  int64 chunk_count = 4;
  int64 file_count = 5;
  google.protobuf.Timestamp created_at = 6;
}

// ── Indexing ──

message IndexRequest {
  string project = 1;
  string root_path = 2;
  repeated string include_patterns = 3;
  repeated string exclude_patterns = 4;
  bool auto_summarize = 5;  // default: true
}

message IndexResponse {
  string run_id = 1;
  int64 files_indexed = 2;
  int64 chunks_created = 3;
  int64 summaries_generated = 4;
  double duration_ms = 5;
}

// ── Search ──

message SearchRequest {
  string project = 1;
  string query = 2;
  int32 max_results = 3;
  SearchTier tier = 4;
  bool include_summaries = 5;  // default: true
  bool include_raw = 6;        // default: true
}

enum SearchTier {
  TIER_BM25 = 0;
  TIER_SEMANTIC = 1;
  TIER_HYBRID = 2;
  TIER_ENTITY = 3;
}

message SearchResult {
  int64 chunk_id = 1;
  string text = 2;
  float score = 3;
  string file_path = 4;
  int32 start_line = 5;
  int32 end_line = 6;
  bool is_summary = 7;
  SummaryInfo summary = 8;
}

message SummaryInfo {
  string summary_text = 1;
  int64 raw_chunk_count = 2;
  float confidence = 3;
}

// ── Context Building ──

message ContextRequest {
  string project = 1;
  string task = 2;
  int32 max_tokens = 3;
  bool prefer_summaries = 4;  // default: true
}

message ContextResponse {
  string context = 1;
  repeated SearchResult sources = 2;
  ContextStats stats = 3;
}

message ContextStats {
  int32 total_tokens = 1;
  int32 raw_chunks_included = 2;
  int32 summary_chunks_included = 3;
  float summary_ratio = 4;  // 0.0 = all raw, 1.0 = all summaries
}

// ── RLM Run ──

message RunRequest {
  string project = 1;
  string task = 2;
  string backend = 3;
  string model = 4;
  RunOptions options = 5;
}

message RunOptions {
  int32 max_depth = 1;
  int32 max_iterations = 2;
  float max_budget_usd = 3;
  float max_timeout_seconds = 4;
  int32 max_tokens = 5;
}

message RunResponse {
  string run_id = 1;
  RunStatus status = 2;
}

enum RunStatus {
  STATUS_PENDING = 0;
  STATUS_RUNNING = 1;
  STATUS_COMPLETED = 2;
  STATUS_FAILED = 3;
  STATUS_CANCELLED = 4;
}

message RunResult {
  string run_id = 1;
  RunStatus status = 2;
  string answer = 3;
  RunStats stats = 4;
}

message RunStats {
  int32 nodes_visited = 1;
  int32 max_depth_reached = 2;
  int32 total_tokens = 3;
  float total_cost_usd = 4;
  double duration_ms = 5;
}

// ── Session Management ──

message CreateSessionRequest {
  string project = 1;
  string title = 2;
}

message SessionInfo {
  string session_id = 1;
  string project = 2;
  string title = 3;
  google.protobuf.Timestamp created_at = 4;
  int32 turn_count = 5;
}

message SessionTurn {
  string query = 1;
  string response = 2;
  google.protobuf.Timestamp timestamp = 3;
}

// ── Summarization (Hierarchical) ──

enum SummaryScope {
  SCOPE_FILE = 0;      // Per-file summary (1 LLM call per file)
  SCOPE_MODULE = 1;    // Per-directory summary
  SCOPE_PROJECT = 2;   // Top-level project summary
}

message SummaryChunk {
  int64 id = 1;
  int64 buffer_id = 2;
  string content = 3;            // summary text (optimized for LLM)
  SummaryScope scope = 4;
  repeated int64 source_chunk_ids = 5;  // parent raw chunk IDs
  string source_hash = 6;        // hash of all source chunks' content
  float confidence = 7;          // 0.0-1.0, how reliable is this summary
  int32 version = 8;
  int32 tokens = 9;
  google.protobuf.Timestamp created_at = 10;
  google.protobuf.Timestamp updated_at = 11;
}

message SummarizeRequest {
  string project = 1;
  bool force_refresh = 2;        // re-summarize even if source_hash matches
  SummaryScope max_scope = 3;    // how deep to summarize (default: PROJECT)
  int32 max_concurrent = 4;      // parallel LLM calls (default: 10)
}

message SummarizeResponse {
  string run_id = 1;
  SummarizeStatus status = 2;
}

message SummarizeProgress {
  string run_id = 1;
  SummaryScope current_scope = 2;
  string current_file = 3;       // file being summarized (if SCOPE_FILE)
  int32 completed = 4;
  int32 total = 5;
  double elapsed_ms = 6;
  string message = 7;            // "Summarizing auth/middleware.rs..."
}

message SummaryStatus {
  string project = 1;
  int64 total_chunks = 2;
  int64 summarized_chunks = 3;
  float coverage_ratio = 4;
  int64 file_summaries = 5;
  int64 module_summaries = 6;
  int64 project_summaries = 7;
  google.protobuf.Timestamp last_updated = 8;
  repeated StaleSummary stale = 9;  // summaries needing refresh
}

message StaleSummary {
  int64 summary_id = 1;
  string file_path = 2;
  string reason = 3;             // "source_hash mismatch", "new chunks added"
}

// ── Server Management ──

message ServerStatus {
  string version = 1;
  int32 uptime_seconds = 2;
  int32 active_runs = 3;
  int32 total_projects = 4;
  int64 total_chunks = 5;
  int64 total_summaries = 6;
  WriteQueueStats write_queue = 7;
  SummarizeStatus summarize = 8;
}

message WriteQueueStats {
  int32 pending_writes = 1;
  int32 batched_last_flush = 2;
  double avg_latency_ms = 3;
}

message SummarizeStatus {
  bool running = 1;
  string current_file = 2;
  int32 files_remaining = 3;
  double estimated_cost_usd = 4;
}

// ── gRPC Service ──

service AragsService {
  // Project management
  rpc CreateProject(CreateProjectRequest) returns (ProjectInfo);
  rpc ListProjects(google.protobuf.Empty) returns (ListProjectsResponse);
  rpc GetProject(google.protobuf.StringValue) returns (ProjectInfo);

  // Indexing
  rpc IndexProject(IndexRequest) returns (IndexResponse);

  // Search
  rpc Search(SearchRequest) returns (SearchResponse);
  rpc BuildContext(ContextRequest) returns (ContextResponse);

  // RLM
  rpc StartRun(RunRequest) returns (RunResponse);
  rpc GetRun(google.protobuf.StringValue) returns (RunResult);
  rpc CancelRun(google.protobuf.StringValue) returns (google.protobuf.Empty);
  rpc StreamRun(google.protobuf.StringValue) returns (stream RunEvent);

  // Sessions
  rpc CreateSession(CreateSessionRequest) returns (SessionInfo);
  rpc ListSessions(google.protobuf.StringValue) returns (ListSessionsResponse);
  rpc GetSession(google.protobuf.StringValue) returns (SessionInfo);
  rpc AddSessionTurn(AddSessionTurnRequest) returns (SessionTurn);

  // Summarization
  rpc TriggerSummarize(SummarizeRequest) returns (SummarizeResponse);
  rpc GetSummaryStatus(google.protobuf.StringValue) returns (SummaryStatus);
  rpc StreamSummarizeProgress(google.protobuf.StringValue) returns (stream SummarizeProgress);

  // Server management
  rpc GetServerStatus(google.protobuf.Empty) returns (ServerStatus);
  rpc StreamEvents(google.protobuf.Empty) returns (stream RunEvent);
}
```

### Generated types

Use `prost-build` in `build.rs` to generate Rust types from `.proto` files. Expose generated types via `pub mod proto { include!(concat!(env!("OUT_DIR"), "/arags.rs")); }`.

---

## New Crate: `arags-server`

**Responsibility:** Long-running server process that owns all state.

### Module Structure

```
arags-server/src/
├── main.rs              # Entry point, signal handling, graceful shutdown
├── config.rs            # Server configuration (TOML)
├── state.rs             # AppState: shared state across handlers
├── grpc/
│   ├── mod.rs           # gRPC service implementation
│   ├── project.rs       # CreateProject, ListProjects, GetProject
│   ├── index.rs         # IndexProject handler
│   ├── search.rs        # Search, BuildContext handlers
│   ├── run.rs           # StartRun, GetRun, CancelRun, StreamRun
│   ├── session.rs       # Session management handlers
│   ├── summarize.rs     # TriggerSummarize, GetSummaryStatus, StreamProgress
│   └── server.rs        # GetServerStatus, StreamEvents
├── write_queue/
│   ├── mod.rs           # WriteQueue: batched write operations
│   └── batch.rs         # Batch flush logic
├── summarizer/
│   ├── mod.rs           # Summarizer: orchestration, background task
│   ├── strategy.rs      # Summarization strategies (per-file, per-module, per-project)
│   ├── cost.rs          # Cost estimation for summarization
│   └── progress.rs      # Progress tracking and streaming
├── commands/
│   ├── up.rs            # arags-server up (start server)
│   ├── down.rs          # arags-server down (graceful shutdown)
│   ├── status.rs        # arags-server status (health + stats)
│   └── logs.rs          # arags-server logs (tracing subscriber)
└── lifecycle.rs         # Server startup, shutdown, signal handling
```

### AppState

```rust
pub struct AppState {
    pub storage: Storage,           // r2d2 pooled, opened once at startup
    pub vector_store: VectorStore,  // usearch, shared async
    pub event_bus: EventBus,        // Singleton, persists across runs
    pub write_queue: WriteQueue,    // Batched write operations
    pub summarizer: Summarizer,     // Background summarization engine
    pub metrics: AragsMetrics,       // Persistent metrics
    pub config: ServerConfig,       // Loaded from TOML
}
```

### Server Lifecycle Commands

O servidor é gerenciado via comandos CLI dedicados:

```bash
# Native
arags-server up                    # foreground (bloqueia terminal)
arags-server up --daemon           # background (detach)
arags-server down                  # graceful shutdown (envia SIGTERM)
arags-server status                # verifica se está rodando + stats
arags-server logs                  # logs estruturados (tracing)
arags-server logs -f               # follow em tempo real
arags-server logs --level debug    # filtrar por nível

# Docker
docker run -d --name arags \\
  -p 50051:50051 \\
  -v arags-data:/data \\
  arags-server:latest
docker stop arags
docker logs -f arags
```

#### Lifecycle Internals

```rust
// main.rs
#[tokio::main]
async fn main() -> Result<()> {
    let config = ServerConfig::load()?;
    let storage = Storage::open_pooled(&config.data_dir, config.pool_size)?;
    let vector_store = VectorStore::open(&config.data_dir).await?;
    let event_bus = EventBus::new(1024);
    let write_queue = WriteQueue::new(storage.clone(), config.flush_interval);
    let summarizer = Summarizer::new(storage.clone(), config.summarizer);

    let state = AppState { storage, vector_store, event_bus, write_queue, summarizer, ... };

    // Run migrations once at startup
    state.storage.run_migrations()?;

    // Start background tasks
    write_queue.start();
    summarizer.start();

    // Write PID file for `arags-server status` / `arags-server down`
    std::fs::write(config.pid_file(), std::process::id().to_string())?;

    // Build gRPC server
    let grpc_service = AragsGrpcService::new(state.clone());
    let addr = config.listen_addr.parse()?;
    let server = tonic::transport::Server::builder()
        .add_service(grpc_service)
        .serve_with_shutdown(addr, shutdown_signal())?;

    info!(addr = %addr, "arags-server listening");
    server.await?;

    // Graceful shutdown
    write_queue.flush_and_stop().await;
    summarizer.stop().await;
    std::fs::remove_file(config.pid_file()).ok();
    Ok(())
}

// Signal handling
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut sigterm = tokio::signal::unix::signal(SignalKind::terminate())
        .expect("failed to install SIGTERM handler");

    tokio::select! {
        _ = ctrl_c => info!("received SIGINT, shutting down"),
        _ = sigterm.recv() => info!("received SIGTERM, shutting down"),
    }
}
```

### Config de Conexão

Ordem de resolução do endereço do servidor:

```
1. .arags/config.toml (local, sobrescreve global)
2. ~/.arags/config.toml (global)
3. ARAGS_SERVER_ADDR env var
4. Fallback: 127.0.0.1:50051
```

#### Config Global (`~/.arags/config.toml`)

```toml
[server]
addr = "127.0.0.1:50051"
# Para servidor remoto:
# addr = "arags.myteam.com:50051"

[server.tls]
enabled = false
# cert_path = "/path/to/cert.pem"
# key_path = "/path/to/key.pem"
```

#### Config Local (`.arags/config.toml`)

```toml
[server]
# Sobrescreve config global para este projeto
addr = "arags.myteam.com:50051"
```

#### Env Var

```bash
export ARAGS_SERVER_ADDR="arags.myteam.com:50051"
arags search "autenticação"  # usa o endereço do env
```

### Write Queue

Batches SQLite writes to avoid single-writer contention:

```rust
pub struct WriteQueue {
    sender: mpsc::UnboundedSender<WriteOp>,
    flush_interval: Duration,  // default: 100ms
}

enum WriteOp {
    InsertChunk(ChunkData),
    InsertRun(StoredRun),
    InsertEvent(RlmEvent),
    UpdateSession(SessionUpdate),
    // ... etc
}
```

Background task drains the queue every `flush_interval` or when batch reaches `max_batch_size` (default: 50). Uses a single writer connection from the pool for all batched writes.

### Summarizer (Hierarchical Incremental)

Runs automatically after indexing completes. **Never sends more than ~8K tokens to the LLM in a single summarization call.**

#### Cascade Flow

```
arags index .  (30K linhas, 150 arquivos)
    │
    ├── 1. Chunking (sem LLM, custo $0)
    │      30K linhas → 600 chunks → SQLite
    │
    ├── 2. Per-file summarization (1 LLM call/arquivo)
    │      150 calls paralelas, ~2-8K tokens cada
    │      Input: 5-20 chunks do mesmo arquivo
    │      Output: 1 file summary (~200-500 tokens)
    │      Custo: ~$2-4
    │
    ├── 3. Per-module summarization (1 LLM call/diretório)
    │      20 calls, ~1-3K tokens cada
    │      Input: file summaries do diretório
    │      Output: 1 module summary (~300-600 tokens)
    │      Custo: ~$0.30
    │
    └── 4. Per-project summarization (1 call final)
           1 call, ~2-5K tokens
           Input: module summaries
           Output: 1 project summary (~500-1000 tokens)
           Custo: ~$0.02
```

#### Cost Analysis (10-50K lines)

| Scenario | Files | Chunks | LLM Calls | Est. Cost | Time |
|----------|-------|--------|-----------|-----------|------|
| 10K lines (small) | ~50 | ~200 | ~60 | ~$0.80 | ~1 min |
| 30K lines (medium) | ~150 | ~600 | ~170 | ~$2.50 | ~3 min |
| 50K lines (large) | ~250 | ~1000 | ~280 | ~$4.00 | ~5 min |

**Custo total por indexação:** ~$0.80-4.00 dependendo do tamanho.
**Paralelizável:** 150 calls de arquivo podem rodar com `max_concurrent_subcalls=10`.

#### Incremental Re-sumarization

Ao re-indexar, apenas arquivos modificados são re-sumarizados:

```
Re-index:
  1. Detectar arquivos com source_hash diferente → ~10 arquivos modificados
  2. Re-sumarizar apenas esses 10 arquivos → ~$0.25
  3. Atualizar summaries dos módulos afetados → ~$0.05
  4. Atualizar summary do projeto → ~$0.02
  Total: ~$0.32 (vs ~$2.50 da indexação completa)
```

#### Dual-Layer Search

Quando alguém busca, retorna ambos os tipos:

```
arags search "autenticação"

Resultados:
  1. [SUMMARY] "módulo auth/ - Gerencia autenticação JWT..."
     → 300 tokens, dá visão geral
  2. [RAW] "auth/middleware.rs:45-80" - fn validate_token()
     → 400 tokens, código específico
  3. [RAW] "auth/handlers.rs:12-35" - fn login()
     → 300 tokens, implementação
```

#### Context Building com Summaries

```
Preferência padrão: prefer_summaries=true

Contexto montado (max_tokens=8000):
  [50% budget] → 10-15 sumários de arquivo/módulo (~4000 tokens)
  [50% budget] → 5-10 chunks brutos mais relevantes (~4000 tokens)

Resultado: contexto rico e denso, não poluído com código irrelevante.
```

---

## Refactor: `arags-storage` (SQLite Connection Pool)

### Changes to `conn.rs`

```rust
pub struct Storage {
    // For server mode: pooled connections
    pool: Option<r2d2::Pool<SqliteConnectionManager>>,
    // For CLI mode: single connection (backward compat)
    sqlite: Option<Arc<Mutex<Connection>>>,
    path: PathBuf,
}

impl Storage {
    /// CLI mode: single connection (backward compatible)
    pub fn open(path: &Path) -> Result<Self> { ... }

    /// Server mode: connection pool
    pub fn open_pooled(path: &Path, max_size: u32) -> Result<Self> { ... }

    /// Internal: get a connection (either from pool or single)
    fn get_conn(&self) -> Result<StorageConn<'_>> { ... }
}

enum StorageConn<'a> {
    Pooled(r2d2::PooledConnection<SqliteConnectionManager>),
    Single(MutexGuard<'a, Connection>),
}
```

### Migration Order

1. Add `r2d2` + `r2d2-sqlite` to `arags-storage/Cargo.toml`
2. Refactor `conn.rs`: add `open_pooled()`, internal `get_conn()` abstraction
3. Move PRAGMAs to connection factory (run on each new pooled connection)
4. Run migrations once at startup before pool creation
5. Update all 11 sub-modules: replace `self.conn().lock()` with `self.get_conn()?`
6. Add explicit transactions for multi-statement operations
7. Update `Bm25Search`: remove `Arc<Mutex<Connection>>`, pull from pool
8. Keep `open()` for CLI backward compatibility

### Files Changed

| File | Change |
|------|--------|
| `conn.rs` | New `Storage` struct with pool support, `get_conn()` abstraction |
| `buffers.rs` | Replace `conn().lock()` with `get_conn()?` (~10 methods) |
| `chunks.rs` | Same (~12 methods) |
| `findings.rs` | Same (~3 methods) |
| `tasks.rs` | Same (~5 methods) |
| `runs.rs` | Same (~10 methods) |
| `entities.rs` | Same (~4 methods) |
| `history.rs` | Same (~5 methods) |
| `patterns.rs` | Same (~3 methods) |
| `cache.rs` | Same (~3 methods) |
| `arags-search/bm25.rs` | Remove `Arc<Mutex<Connection>>`, use pool (~8 methods) |
| `Cargo.toml` | Add `r2d2`, `r2d2-sqlite` |

**~70+ methods touched across ~12 files.**

---

## Refactor: `arags-cli` (Thin Client)

### What Gets Removed

| Component | Reason |
|-----------|--------|
| `solve_task_repl()` | REPL doesn't fit server model |
| `CodeExecutor` | No local code execution |
| `repl.rs` | Entire module deleted |
| Direct `Storage::open()` in commands | CLI no longer touches SQLite directly |
| `--repl` CLI flag | Removed |
| JSON/HTTP serve endpoints | Replaced by gRPC |

### What Gets Added

| Component | Purpose |
|-----------|---------|
| `arags-proto` dependency | Generated protobuf types |
| gRPC client setup | Connect to server at startup |
| Server address config | `~/.arags/config.toml` `[server]` section |

### Command Migration

| Current Command | New Behavior |
|----------------|--------------|
| `arags index <path>` | gRPC `IndexProject` → server indexes |
| `arags search <query>` | gRPC `Search` → server searches |
| `arags run <task>` | gRPC `StartRun` → server executes |
| `arags context <task>` | gRPC `BuildContext` → server builds |
| `arags status` | gRPC `GetServerStatus` |
| `arags session ...` | gRPC session methods |
| `arags serve` | **Becomes primary**: starts the server |
| `arags query` | gRPC `Search` + context assembly |
| `arags history` | gRPC session history |
| `arags cost` | gRPC `GetServerStatus` (cost stats) |

### CLI Config Addition

```toml
# ~/.arags/config.toml (global)
[server]
addr = "127.0.0.1:50051"
# Para servidor remoto:
# addr = "arags.myteam.com:50051"

[server.tls]
enabled = false
# cert_path = "/path/to/cert.pem"
# key_path = "/path/to/key.pem"

# .arags/config.toml (local, sobrescreve global)
[server]
addr = "arags.myteam.com:50051"
```

Ordem de resolução:
1. `.arags/config.toml` (local)
2. `~/.arags/config.toml` (global)
3. `ARAGS_SERVER_ADDR` env var
4. Fallback: `127.0.0.1:50051`

---

## Dual-Layer Data Model

### Raw Chunks (existing)

```sql
CREATE TABLE chunks (
    id INTEGER PRIMARY KEY,
    buffer_id INTEGER NOT NULL,
    content TEXT NOT NULL,        -- raw text
    file_path TEXT,
    start_line INTEGER,
    end_line INTEGER,
    chunk_type TEXT,              -- 'code', 'text', 'markdown'
    language TEXT,
    tokens INTEGER,
    version INTEGER DEFAULT 1,
    source_hash TEXT,             -- SHA-256 of content
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (buffer_id) REFERENCES buffers(id)
);
```

### Summary Chunks (new)

```sql
CREATE TABLE summaries (
    id INTEGER PRIMARY KEY,
    buffer_id INTEGER NOT NULL,
    content TEXT NOT NULL,        -- summary text (optimized for LLM)
    scope TEXT NOT NULL,          -- 'file', 'module', 'project'
    source_chunk_ids TEXT,        -- JSON array of parent chunk IDs
    source_hash TEXT,             -- hash of all source chunks' content
    confidence REAL DEFAULT 0.0, -- 0.0-1.0, how reliable is this summary
    version INTEGER DEFAULT 1,
    tokens INTEGER,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (buffer_id) REFERENCES buffers(id)
);
```

### Schema Migration

```sql
-- Migration 10: Add summaries table (hierarchical)
CREATE TABLE IF NOT EXISTS summaries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    buffer_id INTEGER NOT NULL,
    content TEXT NOT NULL,
    scope TEXT NOT NULL CHECK(scope IN ('file', 'module', 'project')),
    source_chunk_ids TEXT,       -- JSON array of parent chunk IDs
    source_hash TEXT,            -- hash of all source chunks' content
    confidence REAL DEFAULT 0.0, -- 0.0-1.0
    version INTEGER DEFAULT 1,
    tokens INTEGER,
    parent_summary_id INTEGER,   -- for module/project: ID of parent summary
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now')),
    FOREIGN KEY (buffer_id) REFERENCES buffers(id) ON DELETE CASCADE,
    FOREIGN KEY (parent_summary_id) REFERENCES summaries(id) ON DELETE SET NULL
);

CREATE INDEX idx_summaries_buffer ON summaries(buffer_id);
CREATE INDEX idx_summaries_scope ON summaries(scope);
CREATE INDEX idx_summaries_source_hash ON summaries(source_hash);
CREATE INDEX idx_summaries_parent ON summaries(parent_summary_id);
```

### Search Integration

Update `HybridSearch` to search both tables:

```rust
pub async fn search(&self, query: &str, opts: &SearchOptions) -> Result<Vec<HybridResult>> {
    let mut results = Vec::new();

    // Tier 1: BM25 on raw chunks
    if opts.include_raw {
        let raw = self.bm25.search(query, opts.max_results).await?;
        results.extend(raw);
    }

    // Tier 2: BM25 on summaries (faster, smaller set)
    if opts.include_summaries {
        let summaries = self.bm25.search_summaries(query, opts.max_results).await?;
        results.extend(summaries);
    }

    // Tier 3: Semantic (searches raw chunks via vector store)
    if let Some(ref semantic) = self.semantic {
        let sem = semantic.search(query, opts.max_results).await?;
        results.extend(sem);
    }

    // RRF fusion
    self.rrf_fusion(&mut results);
    Ok(results)
}
```

### Context Building with Summaries

```rust
pub fn build_context(&self, task: &str, max_tokens: usize, prefer_summaries: bool) -> String {
    let search_results = self.search(task, &SearchOptions { ... });

    if prefer_summaries {
        // Phase 1: Include summary chunks (dense, token-efficient)
        // Start with project summary (overview), then module, then file
        let summaries: Vec<_> = search_results.iter()
            .filter(|r| r.is_summary)
            .take(max_tokens / 2)  // Use half budget for summaries
            .collect();

        // Phase 2: Fill remaining budget with raw chunks
        let raw: Vec<_> = search_results.iter()
            .filter(|r| !r.is_summary)
            .take(max_tokens / 2)
            .collect();

        format!(
            "## Project Overview\n{}\n\n## Relevant Modules\n{}\n\n## Code Details\n{}",
            summaries.iter().filter(|s| s.scope == "project").format("\n"),
            summaries.iter().filter(|s| s.scope == "module").format("\n"),
            raw.iter().map(|r| format!("### {}:{}\n{}", r.file_path, r.start_line, r.text)).format("\n\n")
        )
    } else {
        // Legacy mode: all raw
        format!("## Context\n{search_results}")
    }
}
```

---

## Write Queue Design

### Purpose

SQLite has a single writer. Under concurrent HTTP/gRPC requests (indexing + search + run persistence), write contention becomes a bottleneck. The write queue batches writes and flushes periodically.

### Implementation

```rust
pub struct WriteQueue {
    sender: mpsc::UnboundedSender<WriteOp>,
    handle: JoinHandle<()>,
}

impl WriteQueue {
    pub fn new(storage: Storage, flush_interval: Duration) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        let handle = tokio::spawn(Self::drain_loop(storage, receiver, flush_interval));
        Self { sender, handle }
    }

    pub fn enqueue(&self, op: WriteOp) {
        let _ = self.sender.send(op);
    }

    async fn drain_loop(
        storage: Storage,
        mut receiver: mpsc::UnboundedReceiver<WriteOp>,
        flush_interval: Duration,
    ) {
        let mut buffer = Vec::with_capacity(64);
        let mut interval = tokio::time::interval(flush_interval);

        loop {
            tokio::select! {
                op = receiver.recv() => {
                    match op {
                        Some(op) => buffer.push(op),
                        None => break,  // channel closed, flush and exit
                    }
                }
                _ = interval.tick() => {
                    if !buffer.is_empty() {
                        Self::flush(&storage, &mut buffer).await;
                    }
                }
            }

            // Flush when batch is full
            if buffer.len() >= 50 {
                Self::flush(&storage, &mut buffer).await;
            }
        }

        // Final flush
        if !buffer.is_empty() {
            Self::flush(&storage, &mut buffer).await;
        }
    }

    async fn flush(storage: &Storage, buffer: &mut Vec<WriteOp>) {
        let conn = storage.get_conn().expect("pool exhausted");
        let tx = conn.unchecked_transaction().expect("transaction");

        for op in buffer.drain(..) {
            match op {
                WriteOp::InsertChunk(chunk) => { /* INSERT */ }
                WriteOp::InsertRun(run) => { /* INSERT */ }
                WriteOp::InsertEvent(event) => { /* INSERT */ }
                WriteOp::UpdateSession(update) => { /* UPDATE */ }
            }
        }

        tx.commit().expect("commit");
    }
}
```

### What Gets Queued

| Operation | Priority | Batch? |
|-----------|----------|--------|
| `InsertChunk` (during indexing) | High | Yes |
| `InsertRun` (after RLM completes) | Medium | Yes |
| `InsertEvent` (during RLM execution) | Low | Yes |
| `UpdateSession` | Medium | Yes |
| `UpdateSummary` | Low | Yes |

### What Doesn't Get Queued (Immediate)

| Operation | Reason |
|-----------|--------|
| `Search` (FTS5 MATCH) | Read-only, no queue needed |
| `BuildContext` | Read-only |
| `GetRun` | Read-only |
| Migration DDL | Must run at startup, before pool |

---

## Dependencies Summary

### New crates

| Crate | Purpose | Key Dependencies |
|-------|---------|-----------------|
| `arags-proto` | Protobuf definitions + generated types | `prost`, `prost-build`, `tonic-build` |
| `arags-server` | Long-running server process | `tonic`, `arags-core`, `arags-storage`, `arags-search`, `arags-embedding`, `arags-memory`, `arags-llm` |

### Updated crates

| Crate | New Dependencies | Changes |
|-------|-----------------|---------|
| `arags-storage` | `r2d2`, `r2d2-sqlite` | Connection pool, dual-layer schema |
| `arags-cli` | `arags-proto`, `tonic` | Thin gRPC client, remove REPL |
| `arags-search` | (none) | Search summaries table |
| `arags-core` | (none) | Remove `solve_task_repl` |

### Removed

| Component | Reason |
|-----------|--------|
| `axum` (from CLI) | Replaced by tonic gRPC |
| `tower-http` (from CLI) | No longer needed |
| `tokio-stream` (from CLI) | tonici has its own streaming |
| `repl.rs` (from core) | REPL removed |
| `CodeExecutor` (from core) | REPL removed |

---

## Implementation Phases

### Phase 1: arags-proto crate (1-2 hours)

1. Create `crates/arags-proto/` with `Cargo.toml`
2. Write `proto/arags.proto` with all message/service definitions
3. Create `build.rs` with `tonic_build` + `prost_build`
4. Verify generation: `cargo check -p arags-proto`
5. Add workspace member

### Phase 2: SQLite connection pool (2-3 hours)

1. Add `r2d2`, `r2d2-sqlite` to `arags-storage/Cargo.toml`
2. Refactor `conn.rs`: add `open_pooled()`, `get_conn()` abstraction
3. Create `Summaries` table schema (migration 10)
4. Update all 11 sub-modules to use `get_conn()?`
5. Add explicit transactions for multi-statement operations
6. Update `Bm25Search` to pull from pool
7. Verify: `cargo test -p arags-storage`
8. Verify: `cargo test -p arags-search`

### Phase 3: arags-server crate (3-4 hours)

1. Create `crates/arags-server/` with `Cargo.toml`
2. Implement `state.rs` (AppState)
3. Implement `write_queue/` (batched writes)
4. Implement `grpc/` handlers (one at a time)
5. Implement `lifecycle.rs` (startup, shutdown)
6. Implement `main.rs` (entry point)
7. Verify: `cargo check -p arags-server`
8. Manual test: start server, run gRPC calls with `grpcurl` or similar

### Phase 4: Summarizer (2-3 hours)

1. Implement `summarizer/strategy.rs` (per-file, per-module, per-project)
2. Implement `summarizer/layers.rs` (dual-layer data model)
3. Implement `summarizer/mod.rs` (orchestration, background task)
4. Integrate with indexing pipeline (auto-summarize after index)
5. Update `HybridSearch` to search summaries
6. Update `BuildContext` to prefer summaries
7. Verify: `cargo test -p arags-server`

### Phase 5: arags-cli refactor (2-3 hours)

1. Add `arags-proto` and `tonic` dependencies
2. Create gRPC client module (`client.rs`)
3. Refactor each command to use gRPC client
4. Remove REPL (`repl.rs`, `CodeExecutor`, `--repl` flag)
5. Remove direct `Storage::open()` from all commands
6. Add `[server]` config section
7. Verify: `cargo test -p arags-cli`
8. Verify: `cargo clippy --workspace`

### Phase 6: Cleanup & Migration (1 hour)

1. Remove `axum` + `tower-http` from CLI (old JSON API)
2. Remove `serve.rs` HTTP handlers (replaced by gRPC)
3. Update workspace `Cargo.toml` (new members)
4. Full test suite: `cargo test --workspace`
5. Full clippy: `cargo clippy --workspace -- -D warnings`
6. Update AGENTS.md with new architecture

---

## Risk Assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| `rusqlite::Connection` is `!Send` | High | r2d2 handles thread affinity internally |
| ~70 methods need lock-removal | Medium | Mechanical refactor, each method is small |
| gRPC learning curve | Medium | tonic is well-documented; start with simple unary RPCs |
| Protobuf build requires `protoc` | Low | Already installed (usearch dependency) |
| Write queue data loss on crash | Low | WAL mode + periodic flush; runs are also persisted at completion |
| Summary quality varies by LLM | Medium | Confidence scoring; use best model for summarization |
| Breaking existing tests | Medium | Phased approach; CLI backward compat via `open()` |
| Summarization cost for large codebases | Medium | Hierarchical approach limits cost; incremental re-sum saves 80% |
| Multi-tenant isolation | Low | Single SQLite with buffer_id partitioning; WAL handles concurrency |

---

## Success Criteria

- [ ] `arags-server up` starts server, `arags-server down` stops it gracefully
- [ ] `arags-server status` shows health + stats (projects, chunks, summaries)
- [ ] CLI connects to server via gRPC and executes all 19 commands
- [ ] SQLite connection pool handles 10+ concurrent requests without contention
- [ ] Write queue batches writes and flushes within 100ms
- [ ] Auto-summarization generates hierarchical summaries (file→module→project)
- [ ] Search returns both raw and summary results with RRF fusion
- [ ] Context building prefers summaries when `prefer_summaries=true`
- [ ] Incremental re-summarization only processes changed files
- [ ] Summarization progress streaming works (grpcurl or client)
- [ ] All existing tests pass (170+ tests)
- [ ] No clippy warnings in new code
- [ ] Config resolution: local → global → env → fallback
- [ ] Docker support: `docker run arags-server:latest` works
