# arags-storage

## O que faz
Camada de persistência do `arags`: SQLite (metadados + FTS5/BM25) com um único DB compartilhado isolado por `buffer_id`, mais um vector store embutido (`usearch`, HNSW single-file, L2). Suporta modo single (CLI) e pooled (servidor). CRUD para buffers, chunks, tasks, findings, history, patterns, entities e qa-cache; backup/verify; e busca semântica por embedding.

> **Removido (plan 019):** as tabelas/código de `runs` (RLM runs), `trajectories`
> e `sessions`/`summaries` **foram removidos** do crate — o servidor é LLM-free
> e não há mais runs de RLM, sessões multi-turn nem camada de summaries
> (legacy), mas não é mais populada server-side.

## Estrutura
- `src/lib.rs` — API pública (`pub use sqlite::Storage`, `pub use lance::{VectorStore, SearchResult, VectorEntry}`), `#![allow(...)]` de lint no nível do crate (pedantic style pré-existente + `cfg(test)`).
- `src/sqlite/conn.rs` — `Storage::open`/`open_exclusive`/`open_pooled`, `apply_pragmas`, `StorageConnection` (Single/Pooled), `pool_stats`, `wal_checkpoint(PASSIVE)` (flush de WAL, plan 020) e `backup` (`VACUUM INTO`)/`verify` (`integrity_check`)/`ensure_fts5_available`/`analyze`. `open_pooled` é **híbrido**: pool r2d2 para escritas (`connection()`) + conexão compartilhada dedicada para os read helpers (`conn()`), válidos nos dois modos.
- `src/sqlite/schema.rs` — `run_migrations` (13 migrations versionadas via `schema_version`), `ANALYZE` pós-migração.
- `src/sqlite/buffers.rs` — `Buffer`/`NewBuffer`, `insert_buffer` (UUIDv7), `get_buffer`/`get_buffer_by_name`/`get_buffer_by_uuid`/`list_buffers`/`ensure_uuids`/`update_buffer_counts`/`delete_buffer`.
- `src/sqlite/chunks.rs` — `Chunk`/`NewChunk`, `insert_chunk`/`get_chunk`/`get_chunk_content`/`insert_chunk_content`/`list_chunks`/`count_chunks`/`refresh_last_accessed`/`chunk_exists_by_hash`/`delete_chunks_for_file`/`get_chunks_last_accessed`.
- `src/sqlite/entities.rs` — `extract_entities` (regex determinístico), `ensure_entities_fts`, `insert_chunk_entities`/`get_chunk_entities`, `search_entities`/`search_entities_all` (BM25 sobre FTS5), `EntityHit`.
- `src/sqlite/cache.rs` — `get_cached_result`/`put_cached_result`/`invalidate_project_cache` (result_cache).
- `src/sqlite/findings.rs` — `Finding`, `insert_finding`/`get_findings_for_task`.
- `src/sqlite/history.rs` — `HistoryEntry`, `insert_history`/`get_history`/`purge_history_before` (retenção `[history] retention_days` do server, plan 020; testado inline).
- `src/sqlite/patterns.rs` — `Pattern`, `insert_pattern`/`get_patterns`.
- `src/sqlite/tasks.rs` — `Task`, `insert_task`/`get_pending_tasks`/`update_task_status`/`complete_task`.

> **Removido (plan 019):** `src/sqlite/runs.rs` e `src/sqlite/nodes.rs` (runs de
> RLM e trajectories) **foram excluídos** do crate. O servidor é LLM-free.
- `src/sqlite/tokens.rs` — **Auth (plan 018):** `AuthTokenRow`/`NewToken`, `create_token`/`revoke_token_by_id`/`revoke_token_by_username`/`revoke_all_tokens`/`list_tokens`, `create_session`/`validate_session` (refresh-token rotation + sessões de curta duração, roles `Admin`/`NonAdmin`; plaintext do refresh nunca é persistido).
- `src/sqlite/qa_cache.rs` — **QA-Cache (plan 017):** `QaCacheRow`/`StoreAnswerInput`/`StoredAnswer`, `question_hash`/`chunk_content_hash` (re-export de `arags_core::qa_cache::chunk_content_hash` — cliente e servidor compartilham a mesma implementação, plan 020), `store_answer` (idempotente/reserve-lock), `get_cached_answer`/`get_qa_by_id`/`get_qa_by_cache_id`/`get_qa_by_rowid`, `mark_qa_stale`/`delete_qa`/`touch_qa`, `mark_stale_by_hashes`, `evict_qa`/`evict_all_qa`/`count_qa`/`all_qa_ids`, `list_qa_hashes_for_buffer`, `invalidate_stale_cache_for_buffer`.
- `src/sqlite/chunks.rs` — `Chunk`/`NewChunk`, `insert_chunk`/...; **adicionei** `get_chunks_with_content` e `chunk_hashes_for_buffer` (usados pela staleness hook do QA-Cache).
- `src/lance/vectors.rs` — `VectorStore` (usearch), `VectorEntry`, `SearchResult`; `open`/`insert_vectors`/`search_similar`/`count`; filtro por `buffer_id` via `filtered_search`; mapa `chunk_id→buffer_id` persistido em `vectors.meta` ao lado de `vectors.usearch`.
- `src/qa_vectors.rs` — `QuestionVectorStore` (usearch, espaço B **dedicado** para perguntas, métrica `Cos`); `open`/`insert`/`delete`/`search`/`clear`; chave = `qa_cache.id`.

## Dependências
- Internas: `arags-core` (hash canônico de chunk compartilhado com o client; plan 020).
- Externas (runtime): `rusqlite` (bundled + vtab, FTS5), `usearch` (HNSW single-file), `r2d2`/`r2d2_sqlite` (pool), `anyhow`, `serde`/`serde_json` (meta do vector store), `sha2`, `zstd`, `chrono`, `tokio` (async), `uuid` (v7), `parking_lot` (Mutex), `regex` (entities), `tracing`.
- Externas (dev): `tempfile`.

## Convenções deste módulo
- Sem `unwrap`/`expect`/`panic` em `src/` (deny do workspace); use `anyhow::Result` + `?`. Os testes em `tests/` carregam `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, ...)]` no topo.
- Modelo single-DB: tudo em `~/.arags/knowledge.db`; isolamento por `buffer_id` em todas as tabelas.
- `VectorStore` é `usearch` single-file: `reserve` antes de `add`, `save` após inserção (persiste índice + `vectors.meta`). Buffer filter é feito por predicado durante o `filtered_search` (o usearch não tem metadados nativos).
- `Storage::open` = single (CLI, lock exclusivo opcional); `open_pooled` = servidor híbrido (WAL + r2d2 p/ escritas concorrentes + shared conn p/ leituras; plan 020 `pool_size`).
- `cargo clippy -p arags-storage --all-targets -- -D warnings` deve passar (allows de pedantic style pré-existente no crate).

## Comandos úteis
```bash
CARGO_BUILD_JOBS=4 cargo check  -p arags-storage --all-targets
CARGO_BUILD_JOBS=4 cargo test   -p arags-storage   # 48 testes (src + tests/)
CARGO_BUILD_JOBS=4 cargo clippy -p arags-storage --all-targets -- -D warnings
```

## Migrations
- `migrations/001_initial.sql` … `migrations/016_add_qa_cache.sql` (16 ao total), aplicadas idempotentemente e versionadas via tabela `schema_version`.
- `001` base (chunks, buffers, tasks, findings, history, patterns); `004` runs/custos; `005` trajectories; `007` result_cache; `008` events; `009` entities; `010` last_accessed_at; `011` UUIDv7 em buffers; `013` server handlers (runs.project/model, chunks_fts); `015` auth (plan 018: `auth_tokens`/`auth_sessions`); `016` QA-Cache (plan 017: `qa_cache` + FTS5 `qa_cache_fts` + triggers).
- `run_migrations` roda `ANALYZE` ao final para planner stats.

## Rules
- Mantenha a API pública estável para consumidores (`Storage`, `VectorStore`, `SearchResult`, `VectorEntry`).
- Todo acesso a vetores é por `buffer_id` (filtro no `filtered_search`); o mapa `vectors.meta` deve ser sempre salvo junto com `vectors.usearch`.
- Novas tabelas entram como migration versionada + `run_migrations`; novos CRUD ficam em módulo dedicado em `src/sqlite/`.
- `insert_chunk`/`insert_chunk_content`/`delete_chunks_for_file` são escritas transacionais por arquivo (chunk + FTS + entities + vectors).
- Backup = `Storage::backup(dest)` (`VACUUM INTO`, destino não pode existir); verificação = `Storage::verify()` (`PRAGMA integrity_check`).
