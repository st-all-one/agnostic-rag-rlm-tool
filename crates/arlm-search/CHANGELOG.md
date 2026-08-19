# Changelog

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
