# TODO — arags-server

> **OBSOLETO (pós planos 019/020):** este TODO descreve a arquitetura pré-refator.
> O `arags-server` agora é um **plano de dados puro, LLM-free**: foram removidos o
> `summarizer`, os `runs` de RLM, as `sessions` e o `events` hub. A
> digestão/sumarização ocorre no cliente (`arags-cli`) via o LLM do usuário. A
> config é o arquivo de host `server.toml` (sem `[llm]`). Veja
> `plan/019-cli-consolidation.md` e `plan/020-config-consolidation.md`. Os itens
> abaixo estão arquivados como histórico.

---

## Gaps Críticos (P0)

### 1. RLM executa no servidor — ✅ CONCLUÍDO
- `src/grpc/runs/engine.rs::spawn_engine` persiste o run e dispara
  `arags_core::run_rlm_engine_with_events` em `tokio::spawn`, faz o bridge de
  eventos (`arags_core::EventBus`) para o `EventHub` e persiste o resultado
  (`store::complete_run`/`store::fail_run`) ao final.

### 2. Summarizer usa LLM configurado — ✅ CONCLUÍDO
- `src/state.rs` injeta `Arc<dyn LlmBackend>` no `AppState`; `src/summarizer/engine.rs`
  usa `self.llm.complete(...)` (LLM real, não `NoopLlm`).

### 3. create_project persiste — ✅ CONCLUÍDO
- `src/grpc/project.rs::handle_create_project` → `store::insert_project` insere na
  tabela `buffers` e retorna o id/uuid reais.

### 4. list_projects retorna dados reais — ✅ CONCLUÍDO
- `handle_list_projects` → `store::list_projects` (`SELECT * FROM buffers`).

### 5. Docker compose atualizado — ✅ CONCLUÍDO
- Criado `docker-compose.server.yml` (porta `50051`, `command: ["up"]`,
  `dockerfile: Dockerfile.server`). O root `docker-compose.yml` pertence ao CLI `arags`.

## Gaps Importantes (P1)

### 6. index_project implementado — ✅ CONCLUÍDO
- `src/grpc/index.rs::handle_index_project` orquestra: `discover_files` →
  `indexing::index_file` (chunking determinístico) → `store::insert_chunk` +
  `insert_chunk_text` + `insert_fts_row` + `insert_entities` → `VectorStore`
  (LanceDB, fallback embedder) → `update_buffer_counts`.

### 7. build_context implementado — ✅ CONCLUÍDO
- `src/grpc/search.rs::handle_build_context` faz BM25 via FTS5 (`chunks_fts` +
  `chunk_texts`) e monta contexto LLM com orçamento de tokens.

### 8. get_project implementado — ✅ CONCLUÍDO
- `handle_get_project` → `store::get_project_by_uuid`/`get_project_by_name`.

### 9. LLM configurado no servidor — ✅ CONCLUÍDO
- `src/config.rs` (`[llm]` com `backend`/`model`/`api_key`/`base_url`) e
  `AppState::build_llm` → `config.build_llm()` injeta o backend nos handlers.

### 10. EventBus no servidor — ✅ CONCLUÍDO
- `src/events.rs::EventHub` (broadcast por run/summarize + catch-all) é campo de
  `AppState` e alimenta `stream_run`/`stream_summarize_progress`/`stream_events`.

## Gaps Menores (P2)

### 11. stream_run — ✅ CONCLUÍDO
- `src/grpc/runs/mod.rs::handle_stream_run` registra canal no `EventHub` e faz
  stream via `BroadcastStream`.

### 12. stream_summarize_progress — ✅ CONCLUÍDO
- `src/grpc/summarize.rs::handle_stream_summarize_progress` (canal por run).

### 13. stream_events — ✅ CONCLUÍDO
- `src/grpc/status.rs::handle_stream_events` entrega todos os eventos do servidor.

### 14. Schema consistente — ✅ CONCLUÍDO
- Handlers usam as colunas reais (`sessions.project_name`, `session_history`,
  `runs` sem `project`, etc.); `store/` mapeia 1:1 com as migrations.

### 15. Search query FTS5 correto — ✅ CONCLUÍDO
- `search.rs` usa `chunks_fts` (FTS5) + `chunk_texts`; queries são sanitizadas
  antes do `MATCH` para evitar injeção.

### 16. TLS — ✅ CONCLUÍDO
- `src/lifecycle.rs` habilita `ServerTlsConfig` quando `tls_cert`+`tls_key` estão
  configurados.

### 17. Healthcheck — ✅ CONCLUÍDO
- Subcomando `arags-server status` (cliente gRPC `GetServerStatus`) valida o
  `HEALTHCHECK CMD arags-server status` do `Dockerfile.server`.

### 18. Summarizer com background task persistente — ✅ CONCLUÍDO
- `src/summarizer/worker.rs::spawn_worker` roda um worker persistente
  (`mpsc::UnboundedReceiver`) que consome `SummarizeJob`s. O `write_queue`
  (batched SQLite writer) era código morto e foi **removido** — a indexação
  persiste diretamente via `store/`.

### 19. Teste do Summarizer compila — ✅ CONCLUÍDO
- `tests/summarizer_tests.rs` (9 testes) compila e passa; o construtor do
  `Summarizer` recebe `(storage, llm, events)` conforme a API atual.

---

## Verificação
- `cargo check -p arags-server` → 0 erros, 0 warnings (do crate).
- `cargo test -p arags-server` → 22 testes passando (6 store + 9 summarizer + 7 indexing).
- Arquivos > 300 linhas: nenhum (split concluído em `store/`, `summarizer/`, `grpc/runs/`).
