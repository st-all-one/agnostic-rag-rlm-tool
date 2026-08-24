# arags-search

## O que faz
Motor de busca híbrida do `arags`: BM25 (FTS5), busca por entidades (regex + FTS5), busca semântica (usearch via `arags-storage`), fusão Reciprocal Rank Fusion (RRF, k=60), decay de saliência opcional e montagem de contexto para LLM com token budget.

> **Limpeza pós-019/020:** o Tier 3 de rerank por LLM e a camada dual-layer
> da tabela `summaries` foram **removidos** — o crate não depende mais de
> `arags-llm`, e o servidor ficou LLM-free também no grafo de dependências.

## Estrutura
- `src/lib.rs` — API pública (re-exports), `#![cfg_attr(test, allow(...))]` de lint no nível do crate (pedantic style pré-existente).
- `src/types.rs` — `SearchTier`, `SearchResult`, `HybridResult`, `ChunkWithText`, `OutputFormat`, `SearchOptions`, `Bm25Result`, `SemanticResult`, `EntityResult`.
- `src/bm25.rs` — `Bm25Search` (FTS5 `chunks_fts`, populate/insert/search por `buffer_id`).
- `src/entity.rs` — `EntitySearch` (regex determinístico + FTS5 `entities_fts` no `arags-storage`).
- `src/semantic.rs` — `SemanticSearch` (usa `arags_storage::VectorStore`/usearch; score = `1/(1+distance)`).
- `src/context.rs` — `build_context`/`build_search_results` (token budget, formatos Prompt/Json/Markdown); `load_chunks` hidrata os chunks vencedores da fusão.
- `src/qa_cache.rs` — `cosine_similarity` (vetores) + `jaccard_similarity` (multisets de chunk ids) — matemática pura usada pela resolução de hit/tier do QA-Cache (plan 017); cobertas por testes unitários.
- `src/decay.rs` — `DecayConfig` (decay exponencial de saliência; helpers `refresh_sql`/`age_hours_sql`).
- `src/hybrid/mod.rs` — `HybridSearch` (campos bm25/entity/semantic/llm_backend/rrf_k/decay; `new`/`with_decay`/`with_llm_backend`/`set_decay`).
- `src/hybrid/rrf.rs` — `rrf_score` + `rrf_fuse` (matemática pura de fusão, sem I/O).
- `src/hybrid/fusion.rs` — `apply_decay`, `search_fts` (BM25), `search_all` (cross-project).
- `src/hybrid/search.rs` — `search` async (orquestração multi-tier: FTS → entity → vector → decay).
- `tests/` — `bm25_test`, `context_test`, `decay_test`, `entity_test`, `hybrid_test`, `semantic_test`, `types_test`.

## Dependências
- Internas: `arags-storage` (metadados + FTS5 + vector store/usearch).
- Externas (runtime): `rusqlite` (via `arags-storage`), `parking_lot` (Mutex), `anyhow`, `tracing`, `serde`/`serde_json`.
- Externas (dev): `tempfile`, `tokio` (testes async).

## Convenções deste módulo
- Sem `unwrap`/`expect`/`panic` em `src/` (deny do workspace); use `anyhow::Result` + `?`. Os testes em `tests/` carregam `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, ...)]` no topo.
- Fusão é por **RRF** (`1/(k+rank+1)`); tiers são composíveis e degradam graciosamente (entity/semantic/LLM caem para BM25 em caso de erro, logando `warn`).
- `cargo clippy -p arags-search --all-targets -- -D warnings` deve passar (allows de pedantic style pré-existente no crate).

## Comandos úteis
```bash
CARGO_BUILD_JOBS=4 cargo check  -p arags-search --all-targets
CARGO_BUILD_JOBS=4 cargo test   -p arags-search   # 56 testes (src + tests/)
CARGO_BUILD_JOBS=4 cargo clippy -p arags-search --all-targets -- -D warnings
```

## Migrations
- N/A — o schema (incluindo `chunks_fts`, `qa_cache_fts`) vive em `arags-storage`. `arags-search` consome as tabelas via `Storage`.

## Rules
- Mantenha a API pública estável: `HybridSearch::{new, search, search_fts, search_all}`, `Bm25Search`, `EntitySearch`, `SemanticSearch`, `build_context`, `build_search_results`, `types::*`.
- Novo tier ou fonte de recall entra como branch em `hybrid/search.rs` (RRF no mapa de scores) — manter o padrão de degradação gracefully.
- Ao alterar `HybridResult`/`SearchResult`, atualizar TODOS os locais de construção (src + `tests/`).
- Busca cross-project (`search_all`) funde BM25 de todos os buffers por RRF e trunca em `top_k`.
