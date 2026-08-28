# Changelog

Todas as mudanças notáveis deste crate são documentadas neste arquivo.

O formato segue [Keep a Changelog](https://keepachangelog.com/pt-BR/1.0.0/),
e o versionamento [SemVer](https://semver.org/lang/pt-BR/).

## [Unreleased]
- Env override `ARAGS_EMBEDDER_MODEL_DIR` sobre `embedder.model_dir`
  (imagens container assam/montam checkpoints sem arquivo de config);
  núcleo puro `with_overrides(addr, data_dir, model_dir)` testável.

### Added — plan 023: Unified Contextual Query (epic `agnostic-rag-rlm-tool-43a9`)

- **Espaço C fundido na query** (`grpc/search.rs`): `summary_search` roda FTS
  (`rlm_fts`, com retry OR para linguagem natural) + passada semântica sobre
  `rlm_vectors` em paralelo e funde com **RRF** (k=60, `rrf_score` público do
  `arags-search`) + normalização min-max. `unify_query` divide o budget com
  `split_summary_budget` — sumários qualificados (`summary_min_score`)
  reivindicam até `[search].summary_ratio` (60% default), chunks mantêm ≥1
  slot. Resposta carrega `summaries: Vec<SummaryHit>`; `TIER_SUMMARY`
  permanece compatível (resultados legacy-shaped + summaries preenchidos).
- **Espaço D anexado** (`exploration/search.rs`): pipeline read-time extraído
  para `search_explorations_core` e reuso na unified query; hits frescos
  entram como `ExplorationRef` compacto (gate de status/grounding intacto).
- **Trust pipeline B/C (`agnostic-rag-rlm-tool-ac7f`)**: `provenance_intact` no hit
  exato/near-hit da QA compara `source_hashes` com hashes atuais dos chunks
  (drift → `mark_qa_stale` → MISS); Phase 4.6 em `grpc/index.rs` marca nós RLM
  stale por hash pós-reindex (`mark_rlm_stale_by_hashes`) — saem da busca até
  reprocesso. Falha de verificação falha aberto (nunca quebra serving).
- **Review gate D (`agnostic-rag-rlm-tool-35a1`)**: `[exploration].require_review`
  coloca persist de não-admins em `pending_review`; busca nunca superficializa;
  novo RPC admin-gated `ReviewExploration` aprova/rejeita.
- **Knobs `[search]`** (`agnostic-rag-rlm-tool-9ff2`): `decay_lambda` (serving decay
  via `chunk_ages_hours`; 0=off), `summary_ratio`, `summary_min_score`,
  `exploration_enabled`, `exploration_limit`.
- **Flush vetorial no shutdown**: `AppState::flush_vector_stores()` persiste os
  três espaços debounced após o graceful shutdown (`lifecycle.rs`).
- Testes novos em `grpc/search/tests.rs` (budget/fusão/gates) e
  `grpc/query_cache/tests.rs` (drift de provenance, near-hit cross-project).

### Fixed

- **Deadlock pré-existente** (`arags-storage::chunks`): `get_chunks_with_content`
  re-travava o mutex da conexão via `get_chunk_content` dentro do closure já
  dono do lock — hang eterno no modo Single quando a provenance tinha chunks.
  Corrigido no storage; descoberto pelo teste `exact_hit_with_drifted_provenance_serves_miss`.
- QA near-hit cross-project leak (`agnostic-rag-rlm-tool-3c84`): guard de projeto +
  staleness antes do Jaccard.
- RLM semantic unscoped (`agnostic-rag-rlm-tool-0764`): hidratação escopada por buffer.
- Decay não servido (`agnostic-rag-rlm-tool-fce3`): `[search].decay_lambda` aplica
  decay exponencial nos scores dos chunks.

### Fixed — bootstrap/startup hang (agnostic-rag-rlm-tool-fa25 / 0631 / 9288 / 4cbe)

- **Vetores órfãos em manutenção (`fa25`):** `maintenance::consolidate`/`decay`
  agora recebem o `VectorStore` de chunks e o repassam a `ConsolidationEngine`
  (`with_vector_store`); chunks removidos por deduplicação/decay também têm seus
  vetores usearch apagados (`delete_chunk_ids_blocking`). Antes, a contagem do
  usearch ficava maior que a do SQLite → o bootstrap fazia `clear()` + re-embed de
  tudo a cada reinício (o "deadlock" de startup). Aplicado ao ticker de
  manutenção (`lifecycle.rs`), ao RPC `TriggerMaintenance` (`grpc/memory.rs`) e ao
  `admin consolidate` (que abre o próprio store).
- **Bootstrap em background (`0631`):** `bootstrap_vector_spaces` agora roda num
  `tokio::spawn` — a porta gRPC é bindada imediatamente e o rebuild (quando há
  divergência) ocorre em segundo plano. Servidor saudável em segundos mesmo
  durante um re-embed de minutos; se a divergência reaparecer, o serving não
  trava.
- **Connect timeout do Ollama (`9288`):** `embedder/ollama.rs::http_post` resolve
  o endereço e usa `TcpStream::connect_timeout(10s)` (antes só tinha read timeout).
  Um Ollama inatingível vira erro rápido em vez de stall infinito no embed do
  bootstrap.
- **`admin consolidate` panic (`4cbe`):** `admin::run` era `fn` e aninhava um
  `tokio::Runtime` dentro do `#[tokio::main]` (pânico "Cannot start a runtime from
  within a runtime"). Agora é `async` e faz `.await` de `run_maintenance`
  diretamente; o subcomando `admin consolidate` funciona para limpeza manual.
- **Verificado em e2e (2026-08-28):** volume com 7382 vetores vs 7264 chunks
  (118 órfãos, resíduo pré-fixo) recuperado por bootstrap em background (saudável
  em ~6s, rebuild de ~6min em segundo plano); restart posterior in-sync em 20ms.


### Added — plan 022: handlers de explorações + hook de índice
- **`grpc/exploration/{mod,search,feedback}.rs`**: `PersistExploration`
  (validação de contrato/caps, resolução path→hash, embed best-effort),
  `SearchExplorations` com pipeline read-time (vetor → recheck de âncoras →
  `confidence_score` → gate `hit_low`/`include_stale`/retired) ordenado por
  confiança;   `GetExplorationById` com body/âncoras/metadata vivos;
  `InvalidateExploration` admin (Stale mantém história, Delete remove
  linha+vetor). O RPC de feedback do consumidor (`FeedbackExploration`) foi
  removido depois por risco sybil (ver `agnostic-rag-rlm-tool-f5f3`).
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

### Changed — embedder fixo all-MiniLM-L6-v2 (BREAKING) — agnostic-rag-rlm-tool-1194
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
