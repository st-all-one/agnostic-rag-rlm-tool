# Estrutura do Projeto — Cargo Workspace

## Visão Geral

O projeto `arlm` (Agnostic RLM) é um Cargo workspace com 7 crates, cada um com responsabilidade única. A separação permite compilação paralela, testes isolados, e binários mínimos.

```
arlm/
├── Cargo.toml                  ← Workspace root
├── crates/
│   ├── arlm-cli/              ← Binário CLI (clap)
│   ├── arlm-core/             ← Engine RLM (planner/solver/synthesizer)
│   ├── arlm-storage/          ← SQLite + LanceDB (persistência)
│   ├── arlm-embedding/        ← Chunking + embedding (candle + BGE-M3)
│   ├── arlm-search/           ← Busca híbrida (BM25 + semântico + RRF)
│   ├── arlm-memory/           ← Sistema de memória externa
│   └── arlm-llm/              ← Abstração de backends LLM
├── docker/
│   ├── Dockerfile             ← Build multi-stage
│   ├── Dockerfile.slim        ← Imagem minimalista
│   └── docker-compose.yml     ← Stack completa
├── benchmarks/
│   ├── benches/
│   │   ├── ingestion.rs       ← Benchmark de ingestão
│   │   ├── search.rs          ← Benchmark de busca
│   │   └── rlm_loop.rs        ← Benchmark do loop RLM
│   └── data/                  ← Dados de teste para benchmarks
├── plan/                      ← Documentos de planejamento
└── tests/
    └── integration/           ← Testes de integração cross-crate
```

## Cargo.toml (Workspace Root)

```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2024"          # Rust 1.97 — Edition 2024 (default)
rust-version = "1.85"
license = "MPL-2.0"
repository = "https://github.com/user/arlm"

# Lints consistentes no workspace inteiro (guia Rust: clippy pedantic)
[lints.workspace]
rust.unsafe_code = "forbid"     # só unsafe onde realmente necessário (embedding mmap)
clippy::pedantic = "warn"
clippy::unwrap_used = "deny"
clippy::expect_used = "deny"
clippy::panic = "deny"

[workspace.dependencies]
# CLI
clap = { version = "4", features = ["derive", "env"] }
clap_complete = "4"

# Storage
rusqlite = { version = "0.31", features = ["bundled", "vtab"] }
lancedb = "0.6"
arrow = "52"
arrow-array = "52"

# Embedding
candle-core = { version = "0.7", features = [] }
candle-transformers = "0.7"
candle-nn = "0.7"
tokenizers = "0.19"

# Search (BM25 via FTS5 — já integrado no rusqlite)

# Async
tokio = { version = "1", features = ["full"] }
futures = "0.3"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Utilities
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
rayon = "1.10"
memmap2 = "0.9"
zstd = "0.13"
sha2 = "0.10"
hex = "0.4"
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }
parking_lot = "0.12"
crossbeam-channel = "0.5"
num_cpus = "1.16"
indicatif = "0.17"
console = "0.15"
mimalloc = "0.1"           # allocator global leve (single-binary embarcado)

# Testing
tempfile = "3"
proptest = "1"
criterion = { version = "0.5", features = ["html_reports"] }

[profile.release]
lto = true
codegen-units = 1
panic = "abort"
strip = true
opt-level = 3

# .cargo/config.toml (ver seção "Build Optimization" abaixo)
# [build]
# rustflags = ["-C", "target-cpu=native"]   # CPU do deploy conhecido → ganho direto
```

## Crate: arlm-cli

**Responsabilidade:** Binário CLI, parsing de argumentos, output formatado.

```
crates/arlm-cli/
├── Cargo.toml
├── src/
│   ├── main.rs              ← Entry point
│   ├── commands/
│   │   ├── mod.rs
│   │   ├── run.rs           ← arlm run "tarefa"
│   │   ├── index.rs         ← arlm index ./projeto
│   │   ├── search.rs        ← arlm search "query"
│   │   ├── query.rs         ← arlm query "pergunta" --project ./x
│   │   ├── context.rs       ← arlm context "tarefa" --project ./x
│   │   ├── status.rs        ← arlm status
│   │   ├── history.rs       ← arlm history
│   │   ├── cost.rs          ← arlm cost --by agent [plan 12]
│   │   ├── session.rs       ← arlm session create/resume [plan 13]
│   │   ├── consolidate.rs   ← arlm consolidate
│   │   └── serve.rs         ← arlm serve (HTTP + SSE + /metrics)
│   ├── output/
│   │   ├── mod.rs
│   │   ├── json.rs          ← Output JSON
│   │   ├── tree.rs          ← Output árvore ASCII
│   │   ├── markdown.rs      ← Output Markdown
│   │   └── prompt.rs        ← Output formatado como prompt LLM
│   ├── live.rs              ← Live tree rendering [plan 14]
│   ├── metrics.rs           ← Métricas Prometheus [plan 14]
│   └── util.rs              ← Helpers de CLI
└── Cargo.toml
```

**Dependências:** `arlm-core`, `arlm-storage`, `arlm-search`, `arlm-memory`, `arlm-llm`, `clap`, `indicatif`, `console`, `prometheus`, `axum`

## Crate: arlm-core

**Responsabilidade:** Engine RLM recursivo (planner/solver/synthesizer). O coração do sistema.

```
crates/arlm-core/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── engine.rs            ← Loop recursivo RLM
│   ├── planner.rs           ← Decisão solve/decompose
│   ├── solver.rs            ← Resolução direta
│   ├── synthesizer.rs       ← Merge de resultados (+ compaction)
│   ├── node.rs              ← RlmNode (árvore recursiva)
│   ├── guardrails.rs        ← Ciclo detection, max depth/branching
│   ├── concurrency.rs       ← mapConcurrent, pool de workers
│   ├── budget.rs            ← RunBudget (USD/tokens/errors/time) [plan 12]
│   ├── events.rs            ← EventBus tipado (RlmEvent) [plan 14]
│   ├── cache.rs             ← ResultCache (dedup de subtasks) [plan 14]
│   ├── trajectory.rs        ← RunTrajectory logging [plan 13]
│   └── types.rs             ← StartRunInput, RlmRunResult, etc.
└── Cargo.toml
```

**Dependências:** `arlm-llm`, `arlm-search`, `arlm-storage`, `anyhow`, `serde`, `tracing`, `tokio`, `futures`, `parking_lot`, `tokio-sync`

## Crate: arlm-storage

**Responsabilidade:** Persistência SQLite (metadados, FTS5, estado) + LanceDB (vetores).

```
crates/arlm-storage/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── sqlite/
│   │   ├── mod.rs
│   │   ├── schema.rs        ← DDL, migrações
│   │   ├── chunks.rs        ← CRUD de chunks
│   │   ├── buffers.rs       ← CRUD de buffers (projetos)
│   │   ├── tasks.rs         ← Fila de tasks para dispatch
│   │   ├── findings.rs      ← Resultados de subagentes
│   │   ├── history.rs       ← Histórico de runs
│   │   ├── runs.rs          ← Runs + custo agregado [plan 12]
│   │   ├── trajectories.rs  ← Trajectória completa [plan 13]
│   │   ├── sessions.rs      ← Sessões multi-turn [plan 13]
│   │   ├── result_cache.rs  ← Cache de resultados [plan 14]
│   │   └── conn.rs          ← Connection pool, WAL setup
│   ├── lance/
│   │   ├── mod.rs
│   │   ├── vectors.rs       ← CRUD de vetores
│   │   ├── index.rs         ← Criação/gerenciamento HNSW
│   │   └── search.rs        ← Busca por similaridade
│   └── transaction.rs       ← Transação dual (SQLite + LanceDB)
└── Cargo.toml
```

**Dependências:** `rusqlite`, `lancedb`, `arrow`, `arrow-array`, `sha2`, `anyhow`, `tracing`, `parking_lot`

## Crate: arlm-embedding

**Responsabilidade:** Chunking de código/texto + geração de embeddings via candle.

```
crates/arlm-embedding/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── chunker/
│   │   ├── mod.rs
│   │   ├── code.rs          ← Chunking AST-aware (10 linguagens)
│   │   ├── text.rs          ← Chunking por parágrafos/sentenças
│   │   ├── markdown.rs      ← Chunking por headings
│   │   ├── recursive.rs     ← Chunking recursivo por tamanho
│   │   └── strategies.rs    ← Trait ChunkingStrategy
│   ├── embedder/
│   │   ├── mod.rs
│   │   ├── bge_m3.rs        ← BGE-M3 via candle
│   │   ├── fallback.rs      ← Embedding determinístico (hash)
│   │   ├── cache.rs         ← Cache de embeddings em SQLite
│   │   └── batch.rs         ← Inferência em lote
│   └── pipeline.rs          ← Pipeline completo: arquivo → chunks → embeddings
└── Cargo.toml
```

**Dependências:** `candle-core`, `candle-transformers`, `candle-nn`, `tokenizers`, `memmap2`, `rayon`, `zstd`, `sha2`, `parking_lot`

## Crate: arlm-search

**Responsabilidade:** Busca híbrida (BM25 via FTS5 + semântico + RRF fusion).

```
crates/arlm-search/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── bm25.rs              ← Busca BM25 via SQLite FTS5 (no arlm-storage)
│   ├── semantic.rs          ← Busca semântica via LanceDB
│   ├── hybrid.rs            ← Fusão RRF
│   ├── context.rs           ← Montagem de contexto para LLM
│   └── types.rs             ← SearchResult, HybridResult
└── Cargo.toml
```

**Dependências:** `arlm-storage`, `arlm-embedding`, `lancedb`, `serde`, `anyhow`

## Crate: arlm-memory

**Responsabilidade:** Sistema de memória externa (multi-projeto, histórico, consolidação).

```
crates/arlm-memory/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── project.rs           ← Gerenciamento de projetos
│   ├── knowledge.rs         ← Base de conhecimento acumulado
│   ├── history.rs           ← Histórico de consultas e análises
│   ├── consolidation.rs     ← Consolidação e limpeza de memória
│   ├── transfer.rs          ← Transferência entre projetos
│   ├── watch.rs             ← Monitoramento de mudanças (inotify)
│   ├── session.rs           ← Sessões multi-turn [plan 13]
│   └── trajectory.rs        ← Reuso de trajectórias [plan 13]
└── Cargo.toml
```

**Dependências:** `arlm-storage`, `arlm-embedding`, `arlm-search`, `notify` (inotify), `chrono`, `serde`, `anyhow`

## Crate: arlm-llm

**Responsabilidade:** Abstração unificada de backends LLM (OpenAI, Anthropic, Ollama, etc).

```
crates/arlm-llm/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── trait_llm.rs         ← Trait LlmBackend
│   ├── openai.rs            ← OpenAI API
│   ├── anthropic.rs         ← Anthropic API
│   ├── ollama.rs            ← Ollama (local)
│   ├── gemini.rs            ← Google Gemini
│   ├── mod.rs               ← Factory get_backend()
│   ├── types.rs             ← CompletionRequest, CompletionResponse
│   ├── retry.rs             ← Retry logic com backoff [plan 12]
│   ├── pricing.rs           ← Pricing table (USD por 1M tokens) [plan 12]
│   ├── limits.rs            ← MODEL_CONTEXT_LIMITS por modelo [plan 13]
│   └── token_counter.rs     ← Contagem de tokens (tiktoken + fallback) [plan 13]
└── Cargo.toml
```

**Dependências:** `reqwest`, `serde`, `serde_json`, `tokio`, `anyhow`, `tracing`, `futures`, `tiktoken-rs`

## Fluxo de Compilação

```
Compilação paralela (cargo build --workspace):

arlm-llm          ←─ (sem deps internas)
    ↓
arlm-storage      ←─ (sem deps internas)
    ↓
arlm-embedding    ←─ (sem deps internas)
    ↓
arlm-search       ←─ arlm-storage, arlm-embedding
    ↓
arlm-memory       ←─ arlm-storage, arlm-embedding, arlm-search
    ↓
arlm-core         ←─ arlm-llm, arlm-search, arlm-memory
    ↓
arlm-cli          ←─ arlm-core, arlm-storage, arlm-search, arlm-memory, arlm-llm
```

## Justificativa das Dependências

| Crate | Dependência | Por quê |
|-------|------------|---------|
| arlm-storage | rusqlite (bundled) | SQLite estático, sem dependência do sistema |
| arlm-storage | lancedb | Vetores + HNSW embedding |
| arlm-embedding | candle-core | Inferência local, sem Python |
| arlm-embedding | memmap2 | Zero-copy I/O para arquivos grandes |
| arlm-embedding | rayon | Paralelismo de dados para chunking |
| arlm-embedding | zstd | Compressão de texto em disco |
| arlm-search | rusqlite (FTS5) | BM25 via FTS5 (já no arlm-storage) |
| arlm-core | tokio | Async para chamadas LLM |
| arlm-core | parking_lot | Mutex/RwLock mais rápido que std |
| arlm-core | tokio-sync | Broadcast channel do EventBus [plan 14] |
| arlm-llm | tiktoken-rs | Contagem precisa de tokens [plan 13] |
| arlm-cli | prometheus | Métricas por agente [plan 14] |
| arlm-cli | axum | HTTP + SSE + /metrics no serve mode [plan 14] |
| arlm-cli | indicatif | Barras de progresso |
| arlm-cli | console | Cores e formatação no terminal |
| arlm-cli | mimalloc | Allocator global leve p/ binary embarcado (guia Rust) |

## Build Optimization (Rust 2024 — guia Rust)

O arlm é **embarcado** e roda num CPU de deploy conhecido — habilitar otimizações
que o guia Rust recomenda para single-binaries:

### `target-cpu=native`

`.cargo/config.toml` — gera código para a microarquitetura real (SSE/AVX para
candle/tokenizers):

```toml
[build]
rustflags = ["-C", "target-cpu=native"]

[target.x86_64-unknown-linux-gnu]
rustflags = ["-C", "target-cpu=native"]

[target.aarch64-unknown-linux-gnu]
rustflags = ["-C", "target-cpu=native"]
```

⚠️ Só usar quando o deploy é num CPU conhecido. Para distribuição genérica,
trocar por `target-cpu=x86-64-v3` (AVX2) ou omitir.

### Allocator Global

`arlm-cli` (e `serve`), seta o allocator `mimalloc` no binary — menos alocação/
fragmentação que glibc, ideal para ingestão de 100MB+ e inferência local:

```rust
// crates/arlm-cli/src/main.rs
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;
```

### Edition 2024 + Lints

- `edition = "2024"` no workspace inteiro (acima).
- `[lints.workspace]` com `clippy::pedantic` + `unsafe_code = "forbid"` —
  força revisão explícita de cada bloco `unsafe` (embeddings com mmap).
- `cargo clippy -- -D warnings` e `cargo fmt --check` no CI.

### Compilação do SQLite bundled

Rusqlite `bundled` com `SQLITE3_FLAGS` (detalhado no plano 06) — flags de
performance do SQLite compiladas estaticamente.
