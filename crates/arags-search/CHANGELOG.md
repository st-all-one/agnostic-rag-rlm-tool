# Changelog

## [Unreleased]

### Changed — plan 023: RRF como família única de fusão
- `rrf_score(rank, k)` agora é **público**: o `arags-server` reusa a mesma
  matemática de fusão do pipeline de chunks na `summary_search` do espaço C
  (RLM summaries), unificando a estratégia documentada em
  [README.md](README.md) (tabela de estratégias por espaço).

### Fixed (plan 021 — determinismo da fusão)
- **`rrf_fuse` com tie-break:** empates de score caíam na ordem aleatória de
  iteração do `HashMap`, fazendo duas queries idênticas devolver resultados em
  ordem diferente. Agora ordena por `score DESC, chunk_id ASC` — saída
  deterministicamente estável.
- Propriedades formalizadas em `tests/rrf_proptest.rs` (proptest, 256 cases):
  determinismo/truncamento/ordem decrescente, preservação de união com
  `top_k` grande, monotonicidade estrita de rank dentro de uma lista e limites
  de score (`Σ 1/(k+rank+1)`).
- `proptest` entra como `[dev-dependencies]` (antes declarado no workspace mas
  nunca usado).
- Testes de `qa_cache.rs` movidos para submódulo-arquivo (`qa_cache/tests.rs`).

### Removed (limpeza pós-019/020) — BREAKING
- **Tier 3 de LLM rerank**: `hybrid/rerank.rs`, `with_llm_backend`, campo
  `llm_backend` e consts `RERANK_*`; `SearchTier::LlmRerank` removido do enum.
- **Camada dual-layer de summaries**: leituras de `summaries`/`get_summary` em
  `hybrid/search.rs`, `fusion.rs` e `context.rs`; campos `is_summary`
  (`HybridResult`/`SearchResult`/`ChunkWithText`) e `summary_scope`.
- Parâmetro morto `storage: Option<&Storage>` de `HybridSearch::search`.
- Dependência `arags-llm` — com isso o `arags-server` não compila mais nenhum
  crate de LLM transitivamente.

### Added
- **QA-Cache similarity (plan 017):** `src/qa_cache.rs` com `cosine_similarity`
  (vetores) e `jaccard_similarity` (overlap de provenance) — matemática pura
  usada pela resolução de hit/tier do cache semântico; coberta por testes
  unitários (`jaccard_half_overlap`, `jaccard_disjoint_is_zero`, etc.).

## [0.3.0] - 2026-08-20

### Added
- **Busca dual-layer em summaries** (gaps #1/#2 do TODO): `HybridSearch::search` e
  `search_all` agora consultam também a tabela `summaries` (via FTS5 `summaries_fts`
  no `arags-storage`) e fundem os hits com RRF; resultados trazem `is_summary=true`.
- `HybridResult` e `SearchResult` ganharam `is_summary` (e `SearchResult` ganha
  `summary_scope`); `build_context`/`build_search_results` resolvem summaries.
- `arags-storage`: migration `014_add_summaries_fts.sql` (FTS5 + triggers de sync) e
  `Storage::{search_summaries, search_summaries_all, get_summary}`.

### Changed
- `hybrid.rs` dividido em `hybrid/{mod,rrf,fusion,search,rerank}.rs` (todos < 300 linhas).
- Semantic search documentado como usearch (não mais LanceDB).
- `cargo clippy -p arags-search --all-targets` limpo.

## [0.2.0] - 2026-08-19

### Added
- `HybridSearch::search_all()` para busca cross-project com RRF fusion
- `build_context()` e `build_search_results()` aceitam `max_tokens: Option<u32>`
- Truncamento inteligente de chunks por budget de tokens (word-count heuristic)
- `apply_token_budget()` mantém chunks de maior score dentro do budget
- `truncate_to_tokens()` para truncar texto por estimativa de tokens

### Changed
- `build_context()` e `build_search_results()` agora aceitam parâmetro `max_tokens`

## [0.1.0] - 2026-08-19

### Added
- BM25 search via SQLite FTS5 com tokenizer porter unicode61
- Semantic search via LanceDB vector store
- RRF (Reciprocal Rank Fusion) com k=60
- Sistema de tiers: fts, entity, vector, llm
- Context assembly para LLM (Prompt/JSON/Markdown)
- Unit tests (26 testes)

### Performance
- FTS5 `detail='column'` em vez de `detail='full'` (~40% menos espaço em disco)
  - Suporta: OR, AND, NOT, queries por coluna
  - Não suporta: frases, NEAR (não necessário para BM25)
