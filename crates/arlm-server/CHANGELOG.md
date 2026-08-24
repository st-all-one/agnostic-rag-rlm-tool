# Changelog

Todas as mudanças notáveis deste crate são documentadas neste arquivo.

O formato segue [Keep a Changelog](https://keepachangelog.com/pt-BR/1.0.0/),
e o versionamento [SemVer](https://semver.org/lang/pt-BR/).

## [Unreleased]

### Adicionado
- Subcomando `status` no binário (`arlm-server status`) que consulta a saúde do
  servidor via gRPC `GetServerStatus` — usado pelo `HEALTHCHECK` do
  `Dockerfile.server`.
- `docker-compose.server.yml` para execução containerizada do servidor (porta
  `50051`, `command: ["up"]`).
- **Auth (plan 018):** `src/auth/mod.rs` (`authenticate`/`require_admin`, roles
  `Admin`/`NonAdmin`) + RPC `AuthRefresh`; handlers que escrevem estado exigem
  `Bearer` válido e `InvalidateCache` exige role `Admin`.
- **QA-Cache (plan 017):** handlers em `grpc/query_cache.rs` — `QueryWithCache`
  (busca híbrida + lookup semântico determinístico no `question_vector_store`),
  `StoreAnswer` (idempotente/reserve-lock, persiste `source_hashes`/`provenance`),
  `GetAnswerById` (lookup direto anti-drift por `cache_id`), `InvalidateCache`
  (`Stale`/`Delete` + `similarity_radius`); `QaCacheConfig` (`[qa_cache]`) e
  worker de eviction LRU em background; hook de staleness em `grpc/index.rs`
  (marca `stale` por hash de chunk no pós-reindex). O servidor **não** invoca
  LLM no QA-Cache (digestão roda no client).

### Alterado
- Reorganização type-driven: `store.rs` (800 linhas) dividido em
  `store/{mod,projects,sessions,runs,chunks,summaries}.rs`; `summarizer/mod.rs`
  dividido em `summarizer/{mod,engine}.rs`; `grpc/runs.rs` dividido em
  `grpc/runs/{mod,engine}.rs`. Nenhum arquivo fonte excede 300 linhas.
- Logs estruturados (`tracing`) e timers (`timing::Timer`) mantidos em todos os
  handlers e operações longas.
- `AppState` agora carrega `llm: Arc<dyn LlmBackend>` e `EventHub` injetados nos
  handlers (RLM real, sumarização com LLM real, streaming de eventos).
- `AppState` agora carrega também `question_vector_store: Option<Arc<QuestionVectorStore>>`
  (espaço B de perguntas, usearch) e `qa_config: QaCacheConfig`; `AppState::new`
  ganha o parâmetro de vector store e dispara o worker de eviction.

### Removido
- Módulo `write_queue` (batched SQLite writer) — código morto, nunca alimentado
  pelos handlers; a indexação persiste diretamente via `store/`.

### Corrigido
- `.map_err(internal)` duplicado em `grpc/project.rs` e `grpc/search.rs`
  (agora usa `store::blocking` de forma consistente).
- `cargo check -p arlm-server` passa com 0 erros e 0 warnings do crate;
  `cargo test -p arlm-server` → 22 testes passando.

### Status do TODO.md
- Todos os gaps (1–19) do `TODO.md` concluídos. O documento original descrevia um
  "shell", mas o crate já implementava a maior parte; o trabalho restante
  (healthcheck, remoção de código morto, splits) está feito.
