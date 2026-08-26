# arags-server

## O que faz
Servidor gRPC long-running da plataforma arags: **plano de dados puro, LLM-free**.
Gerencia projetos (buffers), indexação (chunking + embeddings no servidor +
usearch HNSW), busca híbrida, unified contextual query (plan 023),
memória/histórico, manutenção (consolidate/decay) e
QA-Cache — todas operações determinísticas (sem LLM). A digestão/sumarização
ocorre no cliente (`arags-cli`) via o LLM do usuário.

## Estrutura
- `src/main.rs` — entrypoint; subcomandos `up` (padrão), `status` (healthcheck gRPC), `admin consolidate`.
- `src/lib.rs` — API pública do crate (`ServerConfig`, `AppState`, `run()`).
- `src/config.rs` — `ServerConfig` (TOML host `server.toml`; **sem** `[llm]`):
  listen/TLS/`mtls_ca`, storage (`pool_size`, `flush_interval_ms`,
  `max_batch_size`), `[embedder]` (model_dir/quantization/batch_size/
  max_tokens/overlap_tokens/cache — modelo fixo all-MiniLM-L6-v2), `[search]` (tier/top_k/max_tokens),
  `[qa_cache]`, `[maintenance]`, `[history] retention_days`. Load de
  `ARAGS_SERVER_CONFIG` (default `/etc/arags/server.toml`) + overrides
  `ARAGS_SERVER_ADDR`/`ARAGS_DATA_DIR`/`ARAGS_EMBEDDER_MODEL_DIR`
  (núcleo puro `with_overrides` testável).
- `src/state.rs` — `AppState` (storage, embedder, vector_store, qa_config,
  maintenance config); **plan 023:** espaços vetoriais dedicados
  (`question_vector_store`, `rlm_vector_store`, `exploration_vector_store`) e
  `flush_vector_stores()` — persiste os três espaços debounced via trait
  `FlushableVectorSpace`; `load_embedder(&EmbedderConfig)` constrói o embedder
  nativo all-MiniLM-L6-v2 (`MinilmEmbedder`, INT8 default) a partir de
  `[embedder].model_dir` — hash fallback sem weights — e `wrap_with_cache`
  habilita o `CachedEmbedder` quando `[embedder].cache = true`.
- `src/store/mod.rs` — camada de dados tipada; re-exporta os submódulos.
  - `store/projects.rs` — CRUD de `buffers` + `buffer_id_for_project`.
  - `store/chunks.rs` — chunks, texts, FTS5, entities, contadores de buffer.
  - `store/history.rs` — histórico de consultas por usuário.
  - `store/rlm.rs` — **motor RLM server-side** (pós-index enqueue L1 com
    snapshot de chunks, cascade L2/L3 com tolerância progressiva
    `l2_tolerance`/`l3_tolerance`, agrupamento L2 por primeiro segmento de
    path via `theme_of`, `TEMPLATE_VERSION`); payload único `RlmJobPayload`
    e prioridades nomeadas vindas de `arags_core::rlm`. Testes em
    `store/rlm/tests.rs`.
- `src/grpc/util.rs` — **plan 021:** helpers compartilhados dos handlers —
  `sanitize_fts` (FTS5-safe) e `to_proto_results` (com clamp documentado
  `i64 → i32`); substitui cópias idênticas em `search.rs`/`query_cache.rs`.
- `src/grpc/mod.rs` — dispatcher tonic; um `Timer` por handler.
  - `grpc/project.rs` — create/list/get_project.
  - `grpc/index.rs` — index_project (client-streaming de texto; server chunka
    com `[embedder].max_tokens` e persiste em transações de `max_batch_size`;
    embed em lotes de `[embedder].batch_size`).
  - `grpc/search.rs` — search/context (BM25 FTS5 + semântica + RRF; defaults de
    `[search]` aplicados a tier não informado, top_k e budget de contexto).
    **plan 023 (unified query):** `summary_search` funde FTS+semântica do
    espaço C com RRF (`rrf_score`) e normalização min-max; `unify_query`
    anexa sumários qualificados (`split_summary_budget`: até
    `[search].summary_ratio` do budget, chunks sempre ≥1) e refs de
    explorações (`search_explorations_core`) — tudo best-effort, falha degrada
    para chunk-only; `[search].decay_lambda` aplica decay de saliência no
    serving. Testes em `grpc/search/tests.rs`.
  - `grpc/memory.rs` — `ListMemory`/`GetCache`/`InvalidateCache` (admin).
  - `grpc/history.rs` — histórico de consultas (escopado por refresh token).
  - `grpc/query_cache.rs` — `AuthRefresh` (plan 018) + `QueryWithCache`/
    `StoreAnswer`/`GetAnswerById`/`InvalidateCache` (plan 017); lookup semântico
    determinístico (embed de pergunta no espaço B `question_vector_store`),
    staleness e invalidação (Stale/Delete + raio). **plan 023:**
    `provenance_intact` verifica hashes da provenance antes de servir hit
    (drift → entry stale → MISS); guard projeto+stale no near-hit.
  - `grpc/admin.rs` — `TriggerMaintenance` (consolidate/decay sob demanda).
  - `grpc/status.rs` — get_server_status.
  - `grpc/rlm.rs` — **RPCs de RLM recursive summaries**: claim (lease
    client-supplied, default `DEFAULT_RLM_LEASE_MS`, validação
    1s–1h e `max_level`), **complete (plan 021: transacional via
    `complete_rlm_job_with_node`** — lease/geração + node + job done numa tx;
    admin submete auto-aprovado), job status (poll cooperativo de cancelamento),
    review (admin; rejeição re-enfileira com prioridade elevada) e list nodes.
    Imports explícitos do proto (sem globs).
  - `grpc/exploration/` — **plan 022:** RPCs do dataset de explorações
    (`mod.rs` persist+validação, `search.rs` pipeline read-time,
    `feedback.rs` confirm/contradict + invalidação admin); hook pós-index
    bumpa `project_epochs` e marca mapas stale por âncora. Knobs em
    `[exploration]`; testes em `grpc/exploration/tests.rs` +
    `tests_feedback.rs`. **plan 023:** review gate —
    `[exploration].require_review` manda persist de não-admin para
    `pending_review`; busca nunca superficia pendentes; RPC admin-gated
    `ReviewExploration` aprova (fresh) ou rejeita (retired);
    `search_explorations_core` compartilhado com a unified query.
  - `grpc/error.rs` — mapeamento erro→`Status` (`internal`/`not_found`/...).
- `src/maintenance.rs` — consolidação/decay agendados (cron) + RPC admin.
- `src/indexing.rs` — chunking determinístico (hash, linguagem, classificação).
- `src/lifecycle.rs` — `run`/`run_server`: shutdown gracioso, TLS + mTLS
  (`client_ca_root`), storage pooled híbrido (`pool_size > 1` →
  `Storage::open_pooled`), flusher de WAL (`flush_interval_ms` →
  `wal_checkpoint(PASSIVE)`) e ticker de manutenção com purge de histórico
  (`retention_days`). Abre os vector stores (espaço A/B/C/D) e repassa para
  `AppState::new`; após o graceful shutdown chama
  `state.flush_vector_stores()` para sincronizar os índices HNSW debounced
  com o SQLite (plan 023).
- `src/auth/mod.rs` — `authenticate(MetadataMap, &Storage) -> Result<AuthContext>` +
  `require_admin(&AuthContext)`; roles `Admin`/`NonAdmin` (plan 018).
- `src/qa_vectors` — re-export de `arags_storage::QuestionVectorStore` (espaço B).
- `src/timing.rs` — `Timer` com drop que emite `elapsed_ms`/`elapsed_us`.
- `tests/` — `indexing_tests.rs`, `store_tests.rs`.

## Dependências
- Internas: `arags-core`, `arags-storage`, `arags-search`, `arags-embedding`,
  `arags-memory`, `arags-llm`, `arags-proto`.
- Externas: `tonic`/`prost` (gRPC), `tokio` (async), `rusqlite` (SQLite),
  `futures`, `parking_lot`, `serde`/`toml` (config), `tracing` (logs), `uuid`,
  `sha2`, `chrono`.

## Convenções deste módulo
- Todo acesso SQLite passa por `Storage::connection()` + `conn.execute(closure)`
  (funciona em modo single e pooled) ou por `store::blocking(...)` para I/O bloqueante
  fora do runtime async.
- Handlers são `pub(crate) async fn handle_*` em módulos sob `grpc/`; o `mod.rs`
  apenas faz o dispatch e cria um `Timer`.
- Imports do proto sempre explícitos (`use arags_proto::proto::{TipoA, TipoB};`)
  — globs proibidos (plan 021).
- Testes fora do corpo dos fontes (plan 021): `config/testing.rs`,
  `store/rlm/tests.rs`, `grpc/query_cache/tests.rs`, `grpc/util.rs` (testes
  inline <20 linhas, exceção permitida); integração em `tests/`.
- Nunca use `.unwrap()`/`expect()` em código de produção — `clippy::unwrap_used`/
  `clippy::expect_used` = `deny`.
- Logs estruturados obrigatórios: `tracing::info!(run_id, ...)` com campos tipados.
- Cada operação longa recebe um `timing::Timer` (drop emite tempo de execução).
- Handlers de streaming registram um canal no `EventHub` e convertem `ServerEvent`→proto.

## Comandos úteis
```bash
# Checagem rápida (12 threads)
cargo check -p arags-server

# Testes de integração
cargo test -p arags-server

# Lint
cargo clippy -p arags-server --all-targets

# Rodar o servidor
cargo run -p arags-server -- up

# Healthcheck (precisa de um servidor rodando)
cargo run -p arags-server -- status
```

## Migrations
O schema é gerenciado por `arags-storage` (ver `migrations/` do workspace):
- `buffers` — projetos (id, uuid, name, path, total_chunks, total_files, embedding_model, embedding_dims, ...).
- `chunks` / `chunk_texts` / `chunks_fts` (FTS5) / `chunk_entities` / `entities_fts` — indexação.
- `qa_cache` / `qa_cache_fts` — QA-Cache (plan 017).
- `history` — histórico de consultas por usuário.
- `auth_tokens` / `auth_sessions` — autenticação (plan 018).

## Rules
- `index_project` recebe texto bruto via client-streaming, faz chunking + embeddings
  no servidor, persiste chunks + texts + FTS + entities + vetores (usearch) e só
  então atualiza contadores do buffer.
- `search` sanitiza a query FTS5 antes do `MATCH` (somente alfanuméricos/espaços)
  para evitar injeção.
- `AppState` é `Clone` (Arc internos); a indexação escreve direto via `store/`.
- O servidor **não** possui LLM: não há `summarizer`, `runs` de RLM nem `sessions`.
  A manutenção (consolidate/decay) é disparada por cron ou pelo RPC admin
  `TriggerMaintenance`.
