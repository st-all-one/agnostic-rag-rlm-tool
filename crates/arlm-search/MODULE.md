# arlm-search

## O que faz
Motor de busca híbrida do `arlm`: BM25 (FTS5), busca por entidades (regex + FTS5), busca semântica (usearch via `arlm-storage`), fusão Reciprocal Rank Fusion (RRF, k=60), decay de saliência opcional, rerank por LLM (Tier 3) e montagem de contexto para LLM com token budget. O `HybridSearch` também faz busca **dual-layer**: além dos `chunks`, consulta a tabela `summaries` (FTS5 `summaries_fts` no `arlm-storage`) e marca `is_summary` nos resultados.

## Estrutura
- `src/lib.rs` — API pública (re-exports), `#![cfg_attr(test, allow(...))]` de lint no nível do crate (pedantic style pré-existente).
- `src/types.rs` — `SearchTier`, `SearchResult`, `HybridResult`, `ChunkWithText`, `OutputFormat`, `SearchOptions`, `Bm25Result`, `SemanticResult`, `EntityResult`.
- `src/bm25.rs` — `Bm25Search` (FTS5 `chunks_fts`, populate/insert/search por `buffer_id`).
- `src/entity.rs` — `EntitySearch` (regex determinístico + FTS5 `entities_fts` no `arlm-storage`).
- `src/semantic.rs` — `SemanticSearch` (usa `arlm_storage::VectorStore`/usearch; score = `1/(1+distance)`).
- `src/context.rs` — `build_context`/`build_search_results` (token budget, formatos Prompt/Json/Markdown); `load_chunks` resolve `chunks` **ou** `summaries` conforme `is_summary`.
- `src/qa_cache.rs` — `cosine_similarity` (vetores) + `jaccard_similarity` (multisets de chunk ids) — matemática pura usada pela resolução de hit/tier do QA-Cache (plan 017); cobertas por testes unitários.
- `src/decay.rs` — `DecayConfig` (decay exponencial de saliência; helpers `refresh_sql`/`age_hours_sql`).
- `src/hybrid/mod.rs` — `HybridSearch` (campos bm25/entity/semantic/llm_backend/rrf_k/decay; `new`/`with_decay`/`with_llm_backend`/`set_decay`).
- `src/hybrid/rrf.rs` — `rrf_score` + `rrf_fuse` (matemática pura de fusão, sem I/O).
- `src/hybrid/fusion.rs` — `apply_decay`, `search_fts` (BM25), `search_all` (cross-project + dual-layer summaries).
- `src/hybrid/search.rs` — `search` async (orquestração multi-tier: FTS → entity → vector → decay → LLM rerank; inclui recursão de summaries).
- `src/hybrid/rerank.rs` — `llm_rerank`/`rerank_with_llm` (Tier 3; parse tolerante do ranking do LLM).
- `tests/` — `bm25_test`, `context_test`, `decay_test`, `entity_test`, `hybrid_test` (inclui `test_dual_layer_summaries`), `semantic_test`, `types_test` (56 testes no total).

## Dependências
- Internas: `arlm-storage` (metadados + FTS5 + vector store/usearch), `arlm-llm` (backend de rerank Tier 3).
- Externas (runtime): `rusqlite` (via `arlm-storage`), `parking_lot` (Mutex), `anyhow`, `tracing`, `serde`/`serde_json`.
- Externas (dev): `tempfile`, `tokio` (testes async).

## Convenções deste módulo
- Sem `unwrap`/`expect`/`panic` em `src/` (deny do workspace); use `anyhow::Result` + `?`. Os testes em `tests/` carregam `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, ...)]` no topo.
- Fusão é por **RRF** (`1/(k+rank+1)`); tiers são composíveis e degradam graciosamente (entity/semantic/LLM caem para BM25 em caso de erro, logando `warn`).
- `HybridResult.is_summary` distingue `chunks` de `summaries`; ids de summary e de chunk nunca são mesclados (mapas de score separados em `search`/`search_all`).
- `cargo clippy -p arlm-search --all-targets -- -D warnings` deve passar (allows de pedantic style pré-existente no crate).

## Comandos úteis
```bash
CARGO_BUILD_JOBS=4 cargo check  -p arlm-search --all-targets
CARGO_BUILD_JOBS=4 cargo test   -p arlm-search   # 56 testes (src + tests/)
CARGO_BUILD_JOBS=4 cargo clippy -p arlm-search --all-targets -- -D warnings
```

## Migrations
- N/A — o schema (incluindo `chunks_fts`, `entities_fts`, `summaries_fts`) vive em `arlm-storage` (migration `014_add_summaries_fts.sql` cria/faz sync do FTS5 de summaries). `arlm-search` consome essas tabelas via `Storage`.

## Rules
- Mantenha a API pública estável: `HybridSearch::{new, search, search_fts, search_all}`, `Bm25Search`, `EntitySearch`, `SemanticSearch`, `build_context`, `build_search_results`, `types::*`.
- Todo resultado de busca que venha de `summaries` deve ter `is_summary = true` e (em `SearchResult`) `summary_scope` preenchido; `context.rs::load_chunks` decide a origem (`get_chunk` vs `get_summary`).
- Novo tier ou fonte de recall entra como branch em `hybrid/search.rs` (RRF no mapa de scores) — manter o padrão de degradação gracefully.
- Ao alterar `HybridResult`/`SearchResult`, atualizar TODOS os locais de construção (src + `tests/`); `is_summary` é obrigatório em todo literal.
- Busca cross-project (`search_all`) funde listas de chunks e de summaries separadamente (para não colidir ids) e depois intercala por score.
