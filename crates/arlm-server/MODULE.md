# arlm-server

## O que faz
Servidor gRPC long-running da plataforma arlm: **plano de dados puro, LLM-free**.
Gerencia projetos (buffers), indexação (chunking + embeddings no servidor +
LanceDB), busca híbrida, memória/histórico, manutenção (consolidate/decay) e
QA-Cache — todas operações determinísticas (sem LLM). A digestão/sumarização
ocorre no cliente (`arlm-cli`) via o LLM do usuário.

## Estrutura
- `src/main.rs` — entrypoint; subcomandos `up` (padrão), `status` (healthcheck gRPC), `admin consolidate`.
- `src/lib.rs` — API pública do crate (`ServerConfig`, `AppState`, `run()`).
- `src/config.rs` — `ServerConfig` (TOML host `server.toml`; **sem** `[llm]`).
- `src/state.rs` — `AppState` (storage, embedder, vector_store, qa_config, maintenance config).
- `src/store/mod.rs` — camada de dados tipada; re-exporta os submódulos.
  - `store/projects.rs` — CRUD de `buffers` + `buffer_id_for_project`.
  - `store/chunks.rs` — chunks, texts, FTS5, entities, contadores de buffer.
  - `store/history.rs` — histórico de consultas por usuário.
- `src/grpc/mod.rs` — dispatcher tonic; um `Timer` por handler.
  - `grpc/project.rs` — create/list/get_project.
  - `grpc/index.rs` — index_project (orquestra ingestão; client-streaming de texto).
  - `grpc/search.rs` — search (BM25 FTS5 + semântica + RRF).
  - `grpc/memory.rs` — `ListMemory`/`GetCache`/`InvalidateCache` (admin).
  - `grpc/history.rs` — histórico de consultas (escopado por refresh token).
  - `grpc/query_cache.rs` — `AuthRefresh` (plan 018) + `QueryWithCache`/
    `StoreAnswer`/`GetAnswerById`/`InvalidateCache` (plan 017); lookup semântico
    determinístico (embed de pergunta no espaço B `question_vector_store`),
    staleness e invalidação (Stale/Delete + raio).
  - `grpc/admin.rs` — `TriggerMaintenance` (consolidate/decay sob demanda).
  - `grpc/status.rs` — get_server_status.
  - `grpc/error.rs` — mapeamento erro→`Status` (`internal`/`not_found`/...).
- `src/maintenance.rs` — consolidação/decay agendados (cron) + RPC admin.
- `src/indexing.rs` — chunking determinístico (hash, linguagem, classificação).
- `src/lifecycle.rs` — `run`/`run_server` (shutdown gracioso, TLS opcional); abre o
  `QuestionVectorStore` (espaço B) e repassa para `AppState::new`.
- `src/auth/mod.rs` — `authenticate(MetadataMap, &Storage) -> Result<AuthContext>` +
  `require_admin(&AuthContext)`; roles `Admin`/`NonAdmin` (plan 018).
- `src/qa_vectors` — re-export de `arlm_storage::QuestionVectorStore` (espaço B).
- `src/timing.rs` — `Timer` com drop que emite `elapsed_ms`/`elapsed_us`.
- `tests/` — `indexing_tests.rs`, `store_tests.rs`.

## Dependências
- Internas: `arlm-core`, `arlm-storage`, `arlm-search`, `arlm-embedding`,
  `arlm-memory`, `arlm-llm`, `arlm-proto`.
- Externas: `tonic`/`prost` (gRPC), `tokio` (async), `rusqlite` (SQLite),
  `futures`, `parking_lot`, `serde`/`toml` (config), `tracing` (logs), `uuid`,
  `sha2`, `chrono`.

## Convenções deste módulo
- Todo acesso SQLite passa por `Storage::connection()` + `conn.execute(closure)`
  (funciona em modo single e pooled) ou por `store::blocking(...)` para I/O bloqueante
  fora do runtime async.
- Handlers são `pub(crate) async fn handle_*` em módulos sob `grpc/`; o `mod.rs`
  apenas faz o dispatch e cria um `Timer`.
- Nunca use `.unwrap()`/`expect()` em código de produção — `clippy::unwrap_used`/
  `clippy::expect_used` = `deny`.
- Logs estruturados obrigatórios: `tracing::info!(run_id, ...)` com campos tipados.
- Cada operação longa recebe um `timing::Timer` (drop emite tempo de execução).
- Handlers de streaming registram um canal no `EventHub` e convertem `ServerEvent`→proto.

## Comandos úteis
```bash
# Checagem rápida (12 threads)
cargo check -p arlm-server

# Testes de integração
cargo test -p arlm-server

# Lint
cargo clippy -p arlm-server --all-targets

# Rodar o servidor
cargo run -p arlm-server -- up

# Healthcheck (precisa de um servidor rodando)
cargo run -p arlm-server -- status
```

## Migrations
O schema é gerenciado por `arlm-storage` (ver `migrations/` do workspace):
- `buffers` — projetos (id, uuid, name, path, total_chunks, total_files, embedding_model, embedding_dims, ...).
- `chunks` / `chunk_texts` / `chunks_fts` (FTS5) / `chunk_entities` / `entities_fts` — indexação.
- `qa_cache` / `qa_cache_fts` — QA-Cache (plan 017).
- `history` — histórico de consultas por usuário.
- `auth_tokens` / `auth_sessions` — autenticação (plan 018).

## Rules
- `index_project` recebe texto bruto via client-streaming, faz chunking + embeddings
  no servidor, persiste chunks + texts + FTS + entities + vetores (LanceDB) e só
  então atualiza contadores do buffer.
- `search` sanitiza a query FTS5 antes do `MATCH` (somente alfanuméricos/espaços)
  para evitar injeção.
- `AppState` é `Clone` (Arc internos); a indexação escreve direto via `store/`.
- O servidor **não** possui LLM: não há `summarizer`, `runs` de RLM nem `sessions`.
  A manutenção (consolidate/decay) é disparada por cron ou pelo RPC admin
  `TriggerMaintenance`.
