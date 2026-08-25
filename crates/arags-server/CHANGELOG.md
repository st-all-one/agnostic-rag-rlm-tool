# Changelog

Todas as mudanças notáveis deste crate são documentadas neste arquivo.

O formato segue [Keep a Changelog](https://keepachangelog.com/pt-BR/1.0.0/),
e o versionamento [SemVer](https://semver.org/lang/pt-BR/).

## [Unreleased]

### Added — plan 022: handlers de explorações + hook de índice
- **`grpc/exploration/{mod,search,feedback}.rs`**: `PersistExploration`
  (validação de contrato/caps, resolução path→hash, embed best-effort),
  `SearchExplorations` com pipeline read-time (vetor → recheck de âncoras →
  `confidence_score` → gate `hit_low`/`include_stale`/retired) ordenado por
  confiança; `GetExplorationById` com body/âncoras/metadata vivos;
  `FeedbackExploration` com auto-retire no limite configurado;
  `InvalidateExploration` admin (Stale mantém história, Delete remove
  linha+vetor).
- **Hook pós-index (Phase 4.5)**: `bump_project_epoch` +
  `mark_stale_if_anchors_changed` por projeto indexado.
- **Verify-on-hit (plan 022.8, opcional)**: `[exploration].verify_on_hit`
  embute a afirmação-chave do mapa (`## Conexões`, extraída por
  `claim_text`) e busca contra os vetores de chunk ATUAIS do projeto
  (`cos ≈ 1 − L2²/2`); evidência fraca (`grounding_min_similarity`,
  default 0.25) força `stale` com motivo granular — captura drift semântico
  que âncoras de hash não veem. Teste determinístico com limiar estrito.
- **Config `[exploration]`** (`enabled`, `hit_high`, `hit_low`,
  `max_age_days`, `contradiction_limit`, `verify_on_hit`,
  `grounding_min_similarity`) e `AppState.exploration_vector_store`
  (construtor `with_vector_stores`).
- 4 testes de handler cobrindo validação/unresolved paths, staleness read-time,
  auto-retire por contradições e modos de invalidação.

### Changed (plan 021 — RLM à prova de falha e deduplicação)
- **`handle_complete_rlm_job` transacional:** o handler agora usa
  `Storage::complete_rlm_job_with_node` — validação de lease/geração, upsert do
  node e flip do job para `done` acontecem numa única transação no storage.
  Antes, um job virava `done` antes do node ser persistido; uma falha no meio
  perdia o trabalho voluntário sem retry. O job é carregado **antes** da
  conclusão (proveniência project/level/subject/payload), e `job_id`
  desconhecido responde `NOT_FOUND` imediato.
- **Dedup `sanitize_fts` + `to_proto_results`:** as cópias idênticas em
  `grpc/search.rs` e `grpc/query_cache.rs` foram unificadas em **`grpc/util.rs`**
  (com testes próprios, incluindo Unicode). `to_proto_results` converte linhas
  com `i32::try_from(..).unwrap_or(i32::MAX)` documentado em vez de cast cru.
- `grpc/rlm.rs` importa `DEFAULT_RLM_LEASE_MS` e `RlmJobPayload` de
  `arags_storage::sqlite::rlm` (fonte única); structs locais duplicadas removidas.
- `store/rlm.rs` usa `PRIORITY_FRESH`/`PRIORITY_CASCADE` no lugar de literais.

### Changed (plan 021 — convenções)
- **Zero glob imports do proto:** os 10 handlers com
  `use arags_proto::proto::*;` passaram a importar explicitamente apenas os
  tipos usados (convenção AGENTS.md).
- `qv_store.search(vec, 10)` → const `NEAR_HIT_CANDIDATES`.
- Testes inline de `config.rs`, `store/rlm.rs`, `grpc/query_cache.rs` movidos
  para submódulos-arquivo (`config/testing.rs`, `store/rlm/tests.rs`,
  `grpc/query_cache/tests.rs`) — arquivos de produção enxutos.

### Changed — embedder fixo all-MiniLM-L6-v2 (BREAKING) — agnostic-rlm-rs-1194
- `[embedder]` sem seleção de modelo: `model_dir` (checkpoint MiniLM),
  `quantization` (`int8` default), knobs de chunk/batch/cache. Campos
  `model`, `dims`, `ollama_url/ollama_model/ollama_prefix` **removidos**.
- Sem weights em `model_dir` → hash fallback com aviso (busca semântica
  desligada, pipeline vivo).
- `qa_cache.question_vector_dims` default: 1024 → **384**.
- Reindex obrigatório ao migrar (vetores incompatíveis).

### Removed (limpeza pós-019/020)
- Handlers gRPC de sessão (`grpc/session.rs`) e persistência
  (`store/sessions.rs`) — nenhum cliente chama mais esses RPCs.
- Wrapper de summaries (`store/summaries.rs`) e contagem no `GetServerStatus`;
  status deixou de reportar `total_summaries`.
- Grafo de dependências 100% LLM-free (`arags-llm` nem transitive).

### Added (auditoria plan 020)
- **Schema completo do `server.toml`:** `[embedder]` com
  `model`/`model_dir`/`ollama_url`/`ollama_model`/`ollama_prefix`/`dims`/
  `batch_size`/`quantization`/`cache`; `[search]` (`tier`/`top_k`/`max_tokens`);
  storage tuning (`pool_size`, `flush_interval_ms`, `max_batch_size`);
  `mtls_ca` (mTLS via `client_ca_root`) e `[history] retention_days`.
- **Embedder pela config:** `state::load_embedder(&EmbedderConfig)` substitui as
  envs `ARAGS_MODEL_DIR`/`ARAGS_OLLAMA_*`; `CachedEmbedder` ativado por
  `[embedder].cache` (cache SQLite por hash, degrada sem falhar).
- **Storage híbrido:** `pool_size > 1` abre `open_pooled` (escritas no pool +
  conexão compartilhada p/ leituras); flusher de WAL checkpoint PASSIVE;
  indexação grava em transações de `max_batch_size` linhas
  (`store::insert_chunks_batched`).
- **Purge de histórico** pelo ticker de manutenção (`[history] retention_days`,
  default 90; 0 = mantém).

### Changed (auditoria plan 020)
- Proto `SearchTier` renumerado: `SEARCH_TIER_UNSPECIFIED = 0` (tiers 1–4);
  requests sem tier resolvem para `[search].tier` do `server.toml`.
- `admin create-refresh` aponta para `~/.arags/arags.toml [auth]`.

> **Nota (planos 019/020):** o servidor tornou-se um **plano de dados puro,
> LLM-free**. Foram **removidos** o `summarizer` (server-side), os `runs` de RLM,
> as `sessions` e o `events` hub. A digestão/sumarização agora ocorre no cliente
> (`arags-cli`) via o LLM do usuário. A config passou a ser o arquivo de host
> `server.toml` (lido de `ARAGS_SERVER_CONFIG` ou `/etc/arags/server.toml`), **sem**
> seção `[llm]`. A manutenção (consolidate/decay) é feita por cron + RPC admin
> `TriggerMaintenance`. Veja `plan/019-cli-consolidation.md` e
> `plan/020-config-consolidation.md`.

### Adicionado
- Subcomando `status` no binário (`arags-server status`) que consulta a saúde do
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
- `cargo check -p arags-server` passa com 0 erros e 0 warnings do crate;
  `cargo test -p arags-server` → 22 testes passando.

### Status do TODO.md
- Todos os gaps (1–19) do `TODO.md` concluídos. O documento original descrevia um
  "shell", mas o crate já implementava a maior parte; o trabalho restante
  (healthcheck, remoção de código morto, splits) está feito.
