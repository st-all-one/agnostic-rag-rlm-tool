# Changelog

## [Unreleased]

### Added — plan 022: dataset de explorações

- **Migration `019_add_explorations.sql`**: `explorations` (status
  fresh/stale/retired, epoch de criação, contadores confirm/contradict),
  `exploration_files` (âncoras `content_hash` com roles cited/context),
  `explorations_fts` (FTS5 com triggers) e `project_epochs` (época monotônica
  por projeto).
- **Novo módulo `sqlite/explorations/`** (`store`, `staleness`, `feedback`):
  persist transacional linha+âncoras com body comprimido em zstd; FTS scoped
  por projeto; `bump/current_project_epoch`; `mark_stale_if_anchors_changed`
  (compara âncora citada com hash vigente do chunk, grava `stale_reason`
  granular); `recheck_anchors_for_rowid` para verificação em tempo de leitura;
  `current_hashes_for_paths` para resolução no servidor;
  `record_feedback` (confirm/contradict com auto-retire no limite);
  invalidação admin Stale/Delete auditada.
- **`exploration_vectors.rs`**: espaço vetorial dedicado (usearch cosseno,
  arquivo `exploration_vectors.usearch`, chave = rowid) — terceiro espaço,
  isolado de chunks e perguntas.
- Consts de domínio (`STATUS_*`, `ROLE_*`, `TEMPLATE_VERSION_V1`) agora vivem
  em `arags_core::exploration` e são reexportadas aqui (fonte única).
- Testes: `tests/explorations_storage_test.rs` (10) +
  `tests/exploration_vectors_test.rs` (4, inclui não-interferência dos três
  espaços).

### Changed (plan 021 — hardening e modularização do RLM)
- **`sqlite/rlm.rs` (1001 linhas) dividido em `sqlite/rlm/`**: `mod.rs` (tipos,
  consts re-exportadas do `arags_core::rlm`, mappers, upsert compartilhado),
  `nodes.rs` (CRUD/review gate/FTS/hydration), `jobs.rs` (enqueue/fail/cancel/
  requeue/payload/count), `complete.rs` (claim + conclusões) e `graph.rs`
  (edges, staleness, snapshot de chunks). Nenhum arquivo passa de 300 linhas
  de produção.
- **SQL 100% parametrizado:** `rlm_parent_chain` e `get_approved_rlm_nodes`
  deixam de interpolar listas `IN ({list})` e passam a vincular o array via
  `json_each(?N)` (mesmo padrão já usado em `mark_rlm_stale_by_hashes`);
  `get_approved_rlm_nodes` valida rowids `u64 → i64` com erro explícito.
- **`parse_json_array` não engole mais erro silenciosamente:** JSON malformado
  em colunas de hashes loga `warn!` (com tamanho do raw) antes de tratar como
  vazio.
- `upsert_node_stmt` extraído como helper único usado por `store_rlm_node` e
  pelo novo caminho transacional.

### Added (plan 021)
- **`Storage::complete_rlm_job_with_node(job_id, worker, generation, node)`** —
  completa um job claimed **e** persiste o summary node numa única transação:
  validação de lease/expiração/geração + INSERT do node + flip para `done`.
  Se o insert falha, o rollback mantém o job `claimed` (retry/requeue), então
  trabalho voluntário nunca é perdido por uma submissão parcial. Retorna
  `Ok(None)` quando o caller não é mais o dono (nada é escrito).
- Testes: `tests/rlm_storage_test.rs` (15 testes comportamentais via API
  pública — upsert/review gate, lease/generation, atomicidade da conclusão,
  staleness, parent-chain CTE, round-trip do payload compartilhado).

### Changed (plan 021 — auth/tokens)
- **`revoke_tokens` sem interpolação SQL:** o parâmetro `where_clause: &str`
  virou o enum privado `RevokeBy { Id, Username }`, cujo `match` despacha
  cláusulas **compile-time fixas**; input do usuário segue sempre vinculado.
- `sqlite/tokens.rs` dividido: `tokens/mod.rs` (refresh tokens) +
  `tokens/session.rs` (`create_session`/`validate_session`), re-exportados no
  mesmo caminho público (`sqlite::tokens::{create_session, validate_session}`).

### Changed (plan 021 — testes fora de `src/`)
- Suites movidas para arquivos dedicados conforme a nova convenção do
  AGENTS.md: `tests/qa_cache_storage_test.rs` (4) e `tests/tokens_test.rs`
  (6, inclui revogação por username purgando sessões); `history` e
  `rlm_vectors` viram submódulos-arquivo (`history/tests.rs`,
  `rlm_vectors/tests.rs`). `[dev-dependencies]` ganha `rusqlite` (macro
  `params!` nos testes externos).

### Changed
- `VectorStore::open` default dims: 1024 → **384** (`arags_core::EMBEDDING_DIMS`,
  all-MiniLM-L6-v2); `open_with_dims` segue disponível.

### Removed (limpeza pós-019/020)
- Módulo `sqlite/summaries.rs` (`Summary`, `insert_summary`,
  `search_summaries`, FTS5 `summaries_fts`) e migrations `006_add_sessions`,
  `012_add_summaries`, `014_add_summaries_fts` — tabelas que ninguém escrevia
  nem lia em produção; referências em `013_server_handlers` limpas.

### Added (auditoria plan 020)
- `Storage::wal_checkpoint()` — checkpoint PASSIVE do WAL para o flusher
  `flush_interval_ms` do server.
- `Storage::purge_history_before(cutoff)` — retenção de histórico
  (`[history] retention_days`), com teste unitário.

### Changed (auditoria plan 020)
- `open_pooled` tornou-se **híbrido**: mantém a conexão compartilhada além do
  pool, então `conn()` (read helpers) funciona em ambos os modos — habilita
  `pool_size > 1` no server sem reescrever os read paths.
- `chunk_content_hash` agora é re-export de `arags_core::qa_cache` (fonte única
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
- `cargo clippy -p arags-storage --all-targets -- -D warnings` limpo; testes de
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
- `cargo clippy -p arags-storage --all-targets -- -D warnings` limpo.

## [0.2.0] - 2026-08-19

### Changed
- **BREAKING:** Single database compartilhado (`~/.arags/knowledge.db`) em vez de DB por projeto
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
