# Changelog

## [0.1.0] - 2026-08-19

### Added
- SQLite storage com WAL mode e otimizações de performance
- Schema com 6 tabelas: chunks, buffers, tasks, findings, history, patterns
- FTS5 para busca textual (BM25) com tokenizer porter unicode61
- LanceDB para armazenamento de vetores com índice HNSW
- Migrações SQL com schema_version tracking
- CRUD completo para todas as tabelas
- Unit tests (23 testes)
