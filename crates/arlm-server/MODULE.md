# arlm-server

## O que faz
Servidor gRPC long-running da plataforma arlm: gerencia projetos (buffers), sessões,
runs de RLM, indexação (chunking + embeddings + LanceDB) e sumarização hierárquica,
com streaming de eventos em tempo real para clientes gRPC.

## Estrutura
- `src/main.rs` — entrypoint; subcomandos `up` (padrão) e `status` (healthcheck gRPC).
- `src/lib.rs` — API pública do crate (`ServerConfig`, `AppState`, `run()`).
- `src/config.rs` — `ServerConfig` + `LlmConfig` (TOML, seção `[llm]`).
- `src/state.rs` — `AppState` (storage, llm, events, vector_store, abort signals de runs).
- `src/events.rs` — `EventHub` (broadcast por run/summarize + catch-all).
- `src/store/mod.rs` — camada de dados tipada; re-exporta os submódulos.
  - `store/projects.rs` — CRUD de `buffers` + `buffer_id_for_project`.
  - `store/sessions.rs` — `sessions` + `session_history`.
  - `store/runs.rs` — runs + mapeamento de status proto↔DB.
  - `store/chunks.rs` — chunks, texts, FTS5, entities, contadores de buffer.
  - `store/summaries.rs` — sumários hierárquicos + estatísticas.
- `src/grpc/mod.rs` — dispatcher tonic; um `Timer` por handler.
  - `grpc/project.rs` — create/list/get_project.
  - `grpc/index.rs` — index_project (orquestra ingestão).
  - `grpc/search.rs` — search + build_context (BM25 FTS5).
  - `grpc/runs/mod.rs` — start/get/cancel/stream_run (handlers).
  - `grpc/runs/engine.rs` — spawn do RLM engine + bridge de eventos.
  - `grpc/session.rs` — create/list/get/add_turn.
  - `grpc/summarize.rs` — trigger/get_status/stream_summarize_progress.
  - `grpc/query_cache.rs` — `AuthRefresh` (plan 018) + `QueryWithCache`/
    `StoreAnswer`/`GetAnswerById`/`InvalidateCache` (plan 017); lookup semântico
    determinístico (embed de pergunta com prefixo `search_query:` no espaço B
    `question_vector_store`), staleness e invalidação (Stale/Delete + raio).
  - `grpc/status.rs` — get_server_status + stream_events.
  - `grpc/error.rs` — mapeamento erro→`Status` (`internal`/`not_found`/...).
- `src/summarizer/mod.rs` — `SummarizeJob`, `SummaryResult`, `compute_hash`, `estimate_tokens`.
- `src/summarizer/engine.rs` — `Summarizer` (carrega chunks, chama LLM, persiste).
- `src/summarizer/{cost,progress,strategy,worker}.rs` — custo, progresso, prompt, worker de background.
- `src/indexing.rs` — chunking determinístico offline (hash, linguagem, classificação).
- `src/lifecycle.rs` — `run`/`run_server` (shutdown gracioso, TLS opcional); abre o
  `QuestionVectorStore` (usearch, espaço B) e repassa para `AppState::new`.
- `src/auth/mod.rs` — `authenticate(MetadataMap, &Storage) -> Result<AuthContext>` +
  `require_admin(&AuthContext)`; roles `Admin`/`NonAdmin` (plan 018).
- `src/qa_vectors` — re-export de `arlm_storage::QuestionVectorStore` (espaço B).
- `src/timing.rs` — `Timer` com drop que emite `elapsed_ms`/`elapsed_us`.
- `tests/` — `indexing_tests.rs`, `store_tests.rs`, `summarizer_tests.rs` (22 testes).

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
- `sessions` / `session_history` — sessões e turns (coluna `project_name`, não `project`).
- `runs` — runs RLM (sem coluna `project`; `partial_answer` guarda o resultado).
- `chunks` / `chunk_texts` / `chunks_fts` (FTS5) / `chunk_entities` / `entities_fts` — indexação.
- `summaries` — sumários hierárquicos por scope (file/module/project).

## Rules
- `index_project` persiste chunks + texts + FTS + entities + vetores (LanceDB) e só
  então atualiza contadores do buffer.
- `start_run` persiste o run como `running`, dispara o engine em background e atualiza
  para `completed`/`failed` ao terminar (nunca bloqueia a resposta gRPC).
- `build_context`/`search` sanitizam a query FTS5 antes do `MATCH` (somente alfanuméricos/
  espaços) para evitar injeção.
- `AppState` é `Clone` (Arc internos); o `write_queue` foi removido — a indexação
  escreve direto via `store/`.
