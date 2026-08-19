# Changelog

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
