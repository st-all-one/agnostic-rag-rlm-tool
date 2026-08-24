# Changelog

## [Unreleased]

### Added (auditoria plan 020)
- `Storage::wal_checkpoint()` — checkpoint PASSIVE do WAL para o flusher
  `flush_interval_ms` do server.
- `Storage::purge_history_before(cutoff)` — retenção de histórico
  (`[history] retention_days`), com teste unitário.

### Changed (auditoria plan 020)
- `open_pooled` tornou-se **híbrido**: mantém a conexão compartilhada além do
  pool, então `conn()` (read helpers) funciona em ambos os modos — habilita
  `pool_size > 1` no server sem reescrever os read paths.
- `chunk_content_hash` agora é re-export de `arlm_core::qa_cache` (fonte única
  compartilhada com o client gRPC puro).

### Added
- **QA-Cache (plan 017):** `src/sqlite/qa_cache.rs` — tabela `qa_cache` + FTS5
  `qa_cache_fts` (triggers de sync), com `store_answer` idempotente (reserve-lock
  por `(project, question_hash)`), lookup exato, staleness por `source_hashes`
  (`mark_stale_by_hashes`), eviction LRU ponderado (`evict_qa`/`evict_all_qa`) e
  hooks de invalidação por buffer (`invalidate_stale_cache_for_buffer`).
- **QA-Cache (plan 017):** `src/qa_vectors.rs` — `QuestionVectorStore` (usearch,
  métrica `Cos`, espaço B dedicado a perguntas; chave = `qa_cache.id`) para o
  lookup semântico de cache no servidor.
- **Auth (plan 018):** `src/sqlite/tokens.rs` — `auth_tokens`/`auth_sessions`,
  `create_token`/`create_session`/`validate_session`/`revoke_*`/`list_tokens`
  (refresh-token rotation + sessões de curta duração, roles `Admin`/`NonAdmin`).
- `src/sqlite/chunks.rs`: `get_chunks_with_content` e `chunk_hashes_for_buffer`
  (suportam a staleness hook do QA-Cache no reindex).
- Migrations `015_add_auth.sql` (plan 018) e `016_add_qa_cache.sql` (plan 017).

### Changed
- `cargo clippy -p arlm-storage --all-targets -- -D warnings` limpo; testes de
  integração `tests/qa_cache_test.rs` (8 testes) cobrindo hit/scoping/reserve-lock/
  staleness/eviction/lookup direto/invalidação.

## [0.3.0] - 2026-08-20

### Changed
- **BREAKING (backend de vetores):** LanceDB → `usearch` (single-file HNSW, L2). Remove `lancedb`, `arrow`, `arrow-array` do workspace — build muito mais leve (sem C++/Arrow/Parquet).
- API pública de `VectorStore` inalterada (`open`/`insert_vectors`/`search_similar`/`count`), mas o `create_index()` específico do LanceDB foi removido (usearch constrói o HNSW automaticamente). Buffer filtering agora via `filtered_search` do usearch (mapa `chunk_id → buffer_id` persistido em `vectors.meta`).
- Arquivos grandes divididos: `runs.rs` teve `FlatNode` movido para `nodes.rs`.

### Added
- `summaries.rs`: CRUD hierárquico (`insert_summary`, `get_summaries`, `get_project_summary`, `get_summary_by_source_hash`) — gap #2 do TODO.
- `Storage::backup()` (`VACUUM INTO`), `Storage::verify()` (`PRAGMA integrity_check`), `Storage::ensure_fts5_available()`, `Storage::analyze()` — gap #5/#6 do TODO.
- Testes de modo pooled concorrente e de backup/verify.

### Performance
- `usearch` é ~10x menor e mais rápido que LanceDB para o mesmo workload HNSW.

### Refactor
- Testes inline extraídos de `src/` para `tests/` (padrão do resto do workspace).
- `cargo clippy -p arlm-storage --all-targets -- -D warnings` limpo.

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
