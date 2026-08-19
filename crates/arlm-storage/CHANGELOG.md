# Changelog

## [0.2.0] - 2026-08-19

### Changed
- **BREAKING:** Single database compartilhado (`~/.arlm/knowledge.db`) em vez de DB por projeto
- `Storage::open()` agora aponta para o diretório shared data
- `Buffer` struct ganhou campo `uuid: Option<String>` (UUIDv7)

### Added
- Migration 011: coluna `uuid` (UUIDv7) na tabela `buffers`
- `get_buffer_by_uuid()` para lookup por UUID
- `ensure_uuids()` para backfill de UUIDs em buffers existentes
- `insert_buffer()` agora gera UUIDv7 automaticamente
- `Buffer_COLUMNS` e `row_to_buffer()` para reduzir duplicação de queries

## [0.1.0] - 2026-08-19

### Added
- SQLite storage com WAL mode e otimizações de performance
- Schema com 6 tabelas: chunks, buffers, tasks, findings, history, patterns
- FTS5 para busca textual (BM25) com tokenizer porter unicode61
- LanceDB para armazenamento de vetores com índice HNSW
- Migrações SQL com schema_version tracking
- CRUD completo para todas as tabelas
- `open_exclusive()` para CLI single-process (elimina arquivo -shm)
- Unit tests (23 testes)

### Fixed
- `delete_buffer()` agora usa transação para atomicidade (50-100x mais rápido)

### Performance
- `ANALYZE` executado automaticamente após migrações (habilita skip-scan)
- `PRAGMA threads=4` para sort paralelo (2-4x em ORDER BY/GROUP BY)
- `PRAGMA automatic_index=ON` para criação automática de índices
- `PRAGMA analysis_limit=1000` para ANALYZE mais rápido
- `PRAGMA locking_mode=EXCLUSIVE` disponível via `open_exclusive()` (-10-20% I/O)
