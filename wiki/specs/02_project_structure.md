# Estrutura do Projeto — Cargo Workspace

## Visão Geral

O projeto `arags` (Agnostic RLM) é um Cargo workspace com 7 crates, cada um com responsabilidade única. A separação permite compilação paralela, testes isolados, e binários mínimos.

```
arags/
├── Cargo.toml                  ← Workspace root
├── crates/
│   ├── arags-cli/              ← Binário CLI (clap)
│   ├── arags-core/             ← Engine RLM (planner/solver/synthesizer)
│   ├── arags-storage/          ← SQLite + usearch (persistência)
│   ├── arags-embedding/        ← Chunking + embedding (candle + BGE-M3)
│   ├── arags-search/           ← Busca híbrida (BM25 + semântico + RRF)
│   ├── arags-memory/           ← Sistema de memória externa
│   └── arags-llm/              ← Abstração de backends LLM
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
repository = "https://github.com/user/arags"

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
usearch = "0.6"
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

## Crate: arags-cli

**Responsabilidade:** Binário CLI, parsing de argumentos, output formatado.

```
crates/arags-cli/
├── Cargo.toml
├── src/
│   ├── main.rs              ← Entry point
│   ├── commands/
│   │   ├── mod.rs
│   │   ├── run.rs           ← arags run "tarefa"
│   │   ├── index.rs         ← arags index ./projeto
│   │   ├── search.rs        ← arags search "query"
│   │   ├── query.rs         ← arags query "pergunta" --project ./x
│   │   ├── context.rs       ← arags context "tarefa" --project ./x
│   │   ├── status.rs        ← arags status
│   │   ├── history.rs       ← arags history
│   │   ├── cost.rs          ← arags cost --by agent [plan 12]
│   │   ├── session.rs       ← arags session create/resume [plan 13]
│   │   ├── consolidate.rs   ← arags consolidate
│   │   └── serve.rs         ← arags serve (HTTP + SSE + /metrics)
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

**Dependências:** `arags-core`, `arags-storage`, `arags-search`, `arags-memory`, `arags-llm`, `clap`, `indicatif`, `console`, `prometheus`, `axum`

## Crate: arags-core

**Responsabilidade:** Engine RLM recursivo (planner/solver/synthesizer). O coração do sistema.

```
crates/arags-core/
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

**Dependências:** `arags-llm`, `arags-search`, `arags-storage`, `anyhow`, `serde`, `tracing`, `tokio`, `futures`, `parking_lot`, `tokio-sync`

## Crate: arags-storage

**Responsabilidade:** Persistência SQLite (metadados, FTS5, estado) + usearch (vetores).

```
crates/arags-storage/
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
│   └── transaction.rs       ← Transação dual (SQLite + usearch)
└── Cargo.toml
```

**Dependências:** `rusqlite`, `usearch`, `arrow`, `arrow-array`, `sha2`, `anyhow`, `tracing`, `parking_lot`

## Crate: arags-embedding

**Responsabilidade:** Chunking de código/texto + geração de embeddings via candle.

```
crates/arags-embedding/
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

## Crate: arags-search

**Responsabilidade:** Busca híbrida (BM25 via FTS5 + semântico + RRF fusion).

```
crates/arags-search/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── bm25.rs              ← Busca BM25 via SQLite FTS5 (no arags-storage)
│   ├── semantic.rs          ← Busca semântica via usearch
│   ├── hybrid.rs            ← Fusão RRF
│   ├── context.rs           ← Montagem de contexto para LLM
│   └── types.rs             ← SearchResult, HybridResult
└── Cargo.toml
```

**Dependências:** `arags-storage`, `arags-embedding`, `usearch`, `serde`, `anyhow`

## Crate: arags-memory

**Responsabilidade:** Sistema de memória externa (multi-projeto, histórico, consolidação).

```
crates/arags-memory/
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

**Dependências:** `arags-storage`, `arags-embedding`, `arags-search`, `notify` (inotify), `chrono`, `serde`, `anyhow`

## Crate: arags-llm

**Responsabilidade:** Abstração unificada de backends LLM (OpenAI, Anthropic, Ollama, etc).

```
crates/arags-llm/
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

arags-llm          ←─ (sem deps internas)
    ↓
arags-storage      ←─ (sem deps internas)
    ↓
arags-embedding    ←─ (sem deps internas)
    ↓
arags-search       ←─ arags-storage, arags-embedding
    ↓
arags-memory       ←─ arags-storage, arags-embedding, arags-search
    ↓
arags-core         ←─ arags-llm, arags-search, arags-memory
    ↓
arags-cli          ←─ arags-core, arags-storage, arags-search, arags-memory, arags-llm
```

## Justificativa das Dependências

| Crate | Dependência | Por quê |
|-------|------------|---------|
| arags-storage | rusqlite (bundled) | SQLite estático, sem dependência do sistema |
| arags-storage | usearch | Vetores + HNSW embedding |
| arags-embedding | candle-core | Inferência local, sem Python |
| arags-embedding | memmap2 | Zero-copy I/O para arquivos grandes |
| arags-embedding | rayon | Paralelismo de dados para chunking |
| arags-embedding | zstd | Compressão de texto em disco |
| arags-search | rusqlite (FTS5) | BM25 via FTS5 (já no arags-storage) |
| arags-core | tokio | Async para chamadas LLM |
| arags-core | parking_lot | Mutex/RwLock mais rápido que std |
| arags-core | tokio-sync | Broadcast channel do EventBus [plan 14] |
| arags-llm | tiktoken-rs | Contagem precisa de tokens [plan 13] |
| arags-cli | prometheus | Métricas por agente [plan 14] |
| arags-cli | axum | HTTP + SSE + /metrics no serve mode [plan 14] |
| arags-cli | indicatif | Barras de progresso |
| arags-cli | console | Cores e formatação no terminal |
| arags-cli | mimalloc | Allocator global leve p/ binary embarcado (guia Rust) |

## Build Optimization (Rust 2024 — guia Rust)

O arags é **embarcado** e roda num CPU de deploy conhecido — habilitar otimizações
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

`arags-cli` (e `serve`), seta o allocator `mimalloc` no binary — menos alocação/
fragmentação que glibc, ideal para ingestão de 100MB+ e inferência local:

```rust
// crates/arags-cli/src/main.rs
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
