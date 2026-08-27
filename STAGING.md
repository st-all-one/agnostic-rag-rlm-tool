# STAGING.md — Status, Missing Work & Priority

> Last updated: **2026-08-26**. Companion to the `sd` tracker (`agnostic-rlm-rs-*`).
> Scope: embedding backends + summarizer/digest (client-side) + release/Docker maintainability.
> Architecture context: `arags-server` is a **pure data plane (no server LLM)**; summaries/digest
> happen **on the client** (`arags-cli` `query -qa` digest, `persist` summarize) using the
> **user's local LLM** via `arags-llm` (plans 019/020/023).

---

## 0. TL;DR — estado dos backends de embedding

| Backend | Como ativar | GPU? | Toolchain de build | Estado | Notas |
|---|---|---|---|---|---|
| **candle** (`Minilm`, all-MiniLM-L6-v2, 384d, int8) | default (sem `kind`; precisa `model_dir` com `model.safetensors`) | ❌ CPU | nenhum (Rust puro) | ✅ shipped | Bakeado em `/models` na imagem Docker |
| **ollama** | `kind = "ollama"` + Ollama daemon local | ✅ (daemon) | nenhum (apenas HTTP) | ✅ implementado | Sem build-toolchain; é o path GPU mais simples no binário lançado |
| **llama.cpp** | `--features llamacpp-vulkan` + `kind = "llamacpp"` + GGUF | ✅ (Vulkan) | **Vulkan SDK no build** + device em runtime | ✅ implementado/validado, **OPT-IN** | Self-contained; veja decisão de manutenibilidade abaixo |

**Decisão de manutenibilidade (fechada em `agnostic-rlm-rs-753b`):** o llama.cpp-Vulkan
**NÃO é default**. Motivo: `vulkan` exige o Vulkan SDK no build → quebraria CI, o Docker
Alpine (`rust:1-alpine` não tem cmake/Vulkan) e os binários do GitHub Release (não portáteis,
precisam de device Vulkan em runtime). O binário/Docker lançado usa **candle + Ollama**; quem
quer o binário GPU self-contained **builda com `--features llamacpp-vulkan`** (artefato separado,
issue `agnostic-rlm-rs-2ff6`). `cargo check/clippy --workspace` passam sem cmake/Vulkan.

---

## 1. Feito (revisão)

- **Server-first data plane** (planos 019/020/023): summarizer removido do servidor; digest/summarize
  mudaram para o cliente (`query -qa`, `persist` → `wiki/*.md`).
- **Embedding — 3 backends** (`crates/arags-embedding/src/embedder/`):
  - `MinilmEmbedder` (candle, int8) — default portátil; validado sucesu.
  - `OllamaEmbedder` — `kind=ollama`; validado ~1.03 ms/chunk na iGPU (Ollama `all-minilm:22m`).
  - `LlamaCppEmbedder` (`llama_cpp.rs`) — **implementado + validado E2E**: offload 7/7 camadas
    para `Vulkan0`, embed 384-dim; ~42 chunks/s no Vulkan *fraco do sandbox*, ~1 ms/chunk esperado
    no Radeon 680M (mesmo engine do Ollama). Opt-in (`llamacpp`/`llamacpp-vulkan`). Issue `753b` fechada.
- **Streaming / OOM fix** (Phase 2, `grpc/index.rs`): decode→chunk→insert→embed inline por arquivo;
  validado sucesu = **1819 arquivos / 9141 chunks em 142s**, sem OOM.
- **`position_ids` off-by-one** corrigido (`minilm/model.rs`).
- **Docker** (`docker/Dockerfile`): build **musl estático** → `scratch`; **candle-only** (all-MiniLM
  bakeado em `/models`), **sem Ollama no container**; `server.toml` usa `model_dir = "/models"`.
  Imagem única `arags-server`. `ARAGS_BIN_URL` permite pular o build (release asset musl).
- **CI/release** (`ci.yml`, `release.yml`): `cargo build/test/clippy --workspace` **sem `--features`**
  → candle; lint limpo sem cmake.

> ⚠️ O STAGING anterior (seção "Docker com Ollama") está **obsoleto**: o container não roda Ollama
> (era para o summarizer server-side, agora removido). O embedding do container é candle por padrão.

---

## 2. O que falta — priorizada (com `sd` IDs)

### P0 — Corretude/robustez do artefato lançado (já tracked, não-embedding)
- `agnostic-rlm-rs-f5db` (Critical, bug, **IN PROGRESS**): projeto canônico `.arags.toml` + index-run-id + delete gracioso + conflito de identidade.
  - **Feito (CLI, 2026-08-26):** nome canônico **manual e obrigatório** (`arags init --name`,
    prompt em TTY, erro fora de TTY — nunca derivado do path). Validação rejeita `.`/`..`/path
    absoluto (`is_valid_canonical_name`). O nome canônico agora é a chave enviada ao servidor em
    `index`/`search`/`query`/`explore` (antes era o path), via `resolve_canonical_name`.
  - **Deferido (server-side, atrelado a epics existentes):** (2) index-run-id/epoch por ingestão →
    `c7b1`; (3) delete gracioso de removidos → `8dcc`/`36ae`; (4) detecção de conflito por
    dissimilaridade de sumários → `c7b1`/LLM do agente; (5) alias de buffers legados (`.`/absoluto)
    → migração no storage (`620d`). A porção CLI fechada; sub-partes server seguem nos epics.
 - `agnostic-rlm-rs-b1a0` (High, task, **IMPLEMENTADO 2026-08-27**): **migração 021 — metadados temporais/versionamento** nas tabelas derivativas (`chunks`, `qa_cache`, `rlm_nodes`, `explorations`).
   - **Feito (`crates/arags-storage/migrations/021_temporal_metadata.sql` + `schema.rs`):** adiciona `version INTEGER NOT NULL DEFAULT 1`, `is_active INTEGER NOT NULL DEFAULT 1`, `superseded_by INTEGER` (nulável), `epoch INTEGER NOT NULL DEFAULT 0`, `created_by TEXT` (nulável) e `model TEXT` (nulável) onde faltavam. Linhas existentes são backfilled por `DEFAULT 1` em `is_active` (sem UPDATE separado). `explorations` já tinha `created_by`/`model`/`epoch_created` (019) → **não duplicados**; nela só entraram `version`/`is_active`/`superseded_by`. Índices parciais `WHERE is_active = 1` por coluna de escopo: `idx_chunks_active(buffer_id, file_path)`, `idx_qa_cache_active(project, buffer_id)`, `idx_rlm_nodes_active(project, level, subject)`, `idx_explorations_active(project)`. `MIGRATION_COUNT` passou de 20→21; `run_migrations` idempotente (skip por `schema_version`) e agora emite `tracing::debug!` com a contagem.
   - **Testes (4, `cargo test -p arags-storage --test schema_test`):** `migration_021_columns_present_on_all_tables`, `migration_021_partial_indices_present`, `migration_021_chunk_defaults` (insert via API → `is_active=1, version=1, epoch=0, superseded_by IS NULL`), `migration_021_is_idempotent`.
   - **Desbloqueia `8dcc`/`786a`/`e210`/`1564`/`c7b1`:** colunas `version`/`is_active`/`superseded_by`/`epoch`/`created_by`/`model` são a base para chunks imutáveis, propagação de autoria, superseding e time-travel (786a popula `created_by`/`model`; `8dcc`/`36ae` usam `is_active`/`superseded_by` no delete gracioso; `c7b1`/`e210`/`1564` consomem `epoch`/`version`).
 - `agnostic-rlm-rs-e5d0` (High, **IMPLEMENTADO 2026-08-26**): abortar `IndexProject` limpo quando cliente desconecta (liberar conn/tx).
   - **Feito (`crates/arags-server/src/grpc/index.rs`):** refatorado o loop em `index_stream_loop<S>(state, stream)` genérico (testável; `handle_index_project` só chama `index_stream_loop(state, req.into_inner())`, mantendo a assinatura gRPC). Detecta desconecte por `None`/`Err` em `stream.next()` e retorna prontamente. Embed tasks são coletadas em `EmbedAbortGuard` (abort no `Drop` + `abort_all()` em todo retorno antecipado) — nenhum `spawn_blocking` de embed sobrevive ao handler. Conexões/tx do SQLite são adquiridas e dropadas dentro de `store::blocking` a cada iteração (nenhuma retida entre batches). `#[instrument(skip_all, fields(buffer_id, project))]` + `debug!`/`info!` com `phase` + `elapsed_ms` por fase/batch.
   - **Testes (2, `cargo test -p arags-server --lib`):** `disconnect_after_init_keeps_deferred_delete_pending` (Init + fim do stream → chunks semeados permanecem, delete deferido NÃO roda) e `disconnect_mid_stream_releases_pooled_connection` (Init + 1 File + fim → handler retorna limpo e `Storage::connection()` subsequente ok, sem conn vazada).
   - **Fix do delete deferido (ccc3) MANTIDO:** o `phase0_done` guard continua em vigor — o replace só roda no primeiro `File`.
 - `agnostic-rlm-rs-ccc3` (High, bug, **IMPLEMENTADO 2026-08-26**): desconectar cliente durante index deixa lock/tx aberta que quebra claim RLM até restart.
   - **Causa-raiz (verificada):** o `index_stream_loop` não retém nenhuma `Connection`/`Transaction` entre iterações — todas são adquiridas e dropadas dentro de `store::blocking` a cada fechamento de closure (nenhum `get_conn()` em `let` externo, nenhum hold através de `.await`). O sintoma de lock/tx aberta era o mesmo vazamento de pool resolvido por `e5d0`; este issue prova via teste que o caminho de claim fica limpo.
   - **Feito (`crates/arags-server/src/grpc/index.rs`):** adicionado `warn!` estruturado no encerramento do stream por desconexão (`reason="client_disconnect"`, `elapsed_ms`, `aborted_embed_tasks`) para diagnosticar hold de lock/tx. Teste de aceite `disconnect_mid_index_keeps_rlm_claim_working`: semeia um `rlm_job` PENDING (via `Storage::enqueue_rlm_job`) para o projeto indexado, desconecta o cliente após Init+1 File, e após o `index_stream_loop` retornar chama `state.storage.claim_rlm_job("worker", DEFAULT_RLM_LEASE_MS, None)` e ASSERTA `Ok(Some(job))` (reproduziria o bug se retornasse `Err`/`None`).
     - **Mecanismo reutilizado:** o `EmbedAbortGuard` + escopo de conexão por-iteração de `e5d0` já libera o pool no disconnect; este teste trava o comportamento para que um `claim rlm_job` nunca mais falhe até restart.
  - `agnostic-rlm-rs-20cd` (Critical, bug, **RESOLVIDO POR e5d0 + teste de regressão 2026-08-27**): re-index duplicava chunks/FTS/vectors (crescimento O(2^n)).
    - **Causa-raiz (verificada):** o `index_stream_loop` (`crates/arags-server/src/grpc/index.rs`) já executa o replace destrutivo *deferido* — no primeiro `File` de cada stream ele chama `store::delete_chunks_for_buffer(bid)` (que apaga de `chunks`, `chunks_fts`, `chunk_texts` e `chunk_entities` em `crates/arags-server/src/store/chunks.rs:307`) e depois purga os vetores via `state.vector_store.delete_chunk_ids(...)` (linhas ~211-223). O delete fica atrelado ao `phase0_done` guard — não roda em `Init`, então disconnect-after-Init não purgeia (mantém a semântica de `e5d0`/`ccc3`).
    - **Teste de regressão (`cargo test -p arags-server --lib`):** `reindex_replaces_chunks_without_duplication` — roda o `index_stream_loop` 3x no mesmo `buffer_id` com conteúdo distinto por run ("alpha_marker"/"beta_marker"/"gamma_marker"); após cada run asserta `chunk_count == 1` (não 1+1+1) e que o marcador da run anterior some de `chunk_texts`/`chunks_fts` enquanto o novo aparece. Prova replace sem duplicação / sem O(2^n). Cobertura de vetores fica a cargo do `vector_store.delete_chunk_ids` (AppState de teste usa storage-only; sintoma reportado era no nível SQLite/FTS).
  - `agnostic-rlm-rs-6690` (High, **IMPLEMENTADO 2026-08-26**): o `arags index` saturava todos os núcleos (matmul do candle via `rayon::join` no pool global) e travava `arags search --tier auto` por 90s — issue pai `5124`.
   - **Causa-raiz:** embed de index (Phase-2) e embed de query compartilhavam o pool global do rayon; durante um index grande o candle ocupava 100% dos cores e a query concorrente não conseguia CPU.
   - **Feito (`crates/arags-server/src/`):** `ServerConfig.index_embed_threads` (default `num_cpus-2`, mín 1; override `ARAGS_INDEX_EMBED_THREADS`) cria um **`rayon::ThreadPool` capado** (`AppState.index_embed_pool`). O embed de Phase-2 roda dentro de `pool.install(|| emb.embed_batch(...))`, confinando o matmul do candle a esses cores e deixando o pool global livre para as queries. `active_index_embeds: Arc<AtomicUsize>` sinaliza contenção; o caminho de query (search/summary/qa) emite `debug!` e continua no pool global (nunca trava). `tracing::info!` registra `index_embed_threads` na subida; o `phase2_embed_batch` agora inclui `pool_threads`.
   - **Testes:** `config::testing::test_server_config_index_embed_threads_reserves_cores` (default reserva núcleos, override respeitado); `grpc::index::tests::index_embeds_on_capped_pool_with_lightweight_embedder` (AppState com pool capado + `LightweightEmbedder` + `VectorStore` → vetores com dims/count corretos); `index_embed_backpressure_keeps_query_serving` (index em background no pool capado + 50 query embeds concorrentes no pool global sob timeout 30s — não trava); `load_regression_index_does_not_starve_search` (`#[ignore="load"]`, doc do repro externo).
   - **Desbloqueia `5124`:** a saturação de núcleos é eliminada pelo pool capado; busca concorrente termina < 90s.
 - `agnostic-rlm-rs-5124` (High, blocked → **desbloqueado e resolvido por `6690`**): index sem isolamento saturava 8 núcleos e bloqueava busca online; corrigido pelo `rayon::ThreadPool` capado em `6690` (reserva cores para serving).

### P1 — Features centrais não validadas de ponta a ponta
- `agnostic-rlm-rs-b020` (High, task, **IMPLEMENTADO 2026-08-26**): **Summarizer cliente E2E com LLM local** (`query -qa`/`persist`)
  via gRPC com LLM real do usuário — validar storage, tempo, ausência de `<think>` (antes só simulado).
  - **Caminho real extraído e testável:** `run_ask` (`qa_cache.rs`) agora chama `digest_chunks(rt, backend, prompt, model)`
    e `run_persist` (`persist.rs`) chama `generate_summary(rt, backend, answer_text, provenance, model)` + `write_wiki(...)`.
    Ambas são `pub(crate)` e injetam `&dyn LlmBackend`, então são testáveis sem servidor gRPC vivo nem LLM real.
  - **`digest_chunks(rt, backend: &dyn LlmBackend, prompt: &str, model: &str) -> Result<String>`** (em `qa_cache.rs`):
    monta `CompletionRequest`, chama `backend.complete`, aplica `strip_cot` e retorna o digest limpo.
  - **`generate_summary(rt, backend: &dyn LlmBackend, answer_text: &str, provenance: &str, model: &str) -> Result<String>`**
    (em `persist.rs`): monta o prompt com seções `## Summary / ## Key Findings / ## Related`, chama `complete`, aplica `strip_cot`.
  - **`write_wiki(project, response_id, model, provenance, summary, title, format) -> Result<PathBuf>`** (em `persist.rs`):
    renderiza via `render_wiki` (agora `pub(crate)`), cria `wiki/`, escreve `.md` e imprime o path — prova o "storage".
  - **Instrumentação de tempo:** `tracing::debug!` com `elapsed_ms` + `model` em ambas as chamadas LLM (barato).
  - **Mock backend de teste:** `MockLlmBackend` (novo `commands/test_helpers.rs`, `#[cfg(test)]`) implementa `LlmBackend`
    com respostas fixas; tem modo limpo e modo que embute `<think>leaked reasoning</think>` para provar o `strip_cot`.
  - **Testes automatizados (`#[cfg(test)]` em `qa_cache.rs`/`persist.rs`):** `digest_chunks_strips_cot`,
    `digest_chunks_clean_passthrough`, `digest_chunks_timing_event_compiles`, `generate_summary_strips_cot_and_has_sections`,
    `render_wiki_includes_summary`, `write_wiki_creates_md_file_under_wiki_dir`, `write_wiki_fulljson_prints_path`
    — cobrem (1) produção+armazenamento, (2) tempo via `debug!`, (3) ausência de `<think>` no output.
  - **Teste E2E com LLM real `#[ignore="requires a configured local LLM..."]`** em `commands/e2e.rs`: só roda se
    `ARAGS_TEST_REAL_LLM=1`, monta backend real de `~/.arags/arags.toml` e asserta sem `<think>` (não roda em CI).
  - **Honestidade:** os testes automatizados exercitam a lógica cliente extraída + backend mock + escrita de arquivo;
    **não** sobem um servidor gRPC in-process (inviável sem o server completo), então o gRPC em si é documentado pelo
    teste `#[ignore]` com LLM real. `cargo clippy --workspace -D warnings`, `cargo fmt --check` e `cargo test -p arags-cli` passam.
- `agnostic-rlm-rs-110e` (Med, bug): **CoT stripping** defensivo em resposta do LLM no digest/summarize
  do cliente (remover `<think>`); + teste unitário.
  - Adicionado `crate::llm_post::strip_cot` (módulo novo `crates/arags-cli/src/llm_post.rs`):
    remove `<think>...</think>` (case-insensitive, com/sem espaço, múltiplos blocos,
    não-terminado vai até EOF) e colapsa whitespace; nunca panica.
  - Aplicado em `qa_cache.rs` (`run_ask`, digest) e `persist.rs` (`run_persist`, summary)
    antes de enviar/escrever; `tracing::debug!` quando CoT é removido (chars removidos).
  - Testes unitários em `llm_post.rs` `#[cfg(test)] mod tests` cobrem: texto limpo,
    bloco único, múltiplos, maiúsculas, não-terminado, newlines, palavra "think" literal,
    parcial (`hello<think> x`→`hello`).
- `agnostic-rlm-rs-7aa8` (Med): repensar superfície `search` vs `query`→`ask` (já tracked).

### P2 — Qualidade / performance / validação
- `agnostic-rlm-rs-6d44` (Med): **Embedding A/B de relevância** all-minilm (384d) vs
  `qwen3-embedding:0.6b` (1024d) em queries NL sobre corpus de código (não só latência).
  - **Módulo de métricas** (`crates/arags-embedding/src/embedder/ab_metrics.rs`): funções puras e
    `#[must_use]` — `cosine_similarity` (guard de dim mismatch → `0.0`, sem pânico),
    `recall_at_k(ranked, relevant, k)`, `ndcg_at_k(ranked, relevant_grades, k)` (relevância graduada;
    binária ok) e `mrr(ranked_list, relevant_list)`. Todas unit-testadas por hand-calc.
  - **Runner A/B in-memory** `run_ab<A: Embedder, B: Embedder>(corpus, queries, ea, eb, k) -> AbResult`
    (campos `recall_a/b, ndcg_a/b, mrr_a/b`): embeda o corpus com **ambos** os embedders (dois espaços
    independentes, dims diferentes ok), embeda cada query, ranqueia por cosseno e calcula as métricas
    vs os chunk ids relevantes; `debug!` loga `elapsed_ms` por query embed e total. Sem servidor/SQLite.
  - **Teste determinístico** `test_run_ab_deterministic`: corpus de 6 chunks + queries NL com ids
    relevantes conhecidos, roda `run_ab` com **dois `LightweightEmbedder`** (sem modelo externo) e
    asserta métricas finitas, em `[0,1]` e `recall_a == recall_b` (prova o wiring E2E sem rede/pesos).
  - **Teste gated `#[ignore]`** `test_ab_real_models_gated`: roda **só** se `ARAGS_AB_B_MODEL` estiver
    set; embedder A = all-minilm (via `ARAGS_MINILM_DIR`, ou `LightweightEmbedder` se ausente) através
    de `build_embedder`, embedder B = Ollama(`ARAGS_AB_B_MODEL`); imprime a comparação. Sem rede no
    default (`cargo test` fica verde). **Como rodar** (humano, Ollama ativo):
    `ARAGS_AB_B_MODEL="qwen3-embedding:0.6b" ARAGS_MINILM_DIR=/models cargo test -p arags-embedding -- --ignored test_ab_real_models_gated`.
  - **Decisão de Docker-bake (modelo + task prefix)** **deferida** até os resultados reais: depende de
    um humano rodar o teste gated acima com Ollama + `qwen3-embedding:0.6b` e comparar as métricas.
    Dockerfiles **não** foram tocados nesta issue.
- `agnostic-rlm-rs-a884` (Med): **Ignores de índice** (`Seeds/`, `storage/logs`, `REFERENCE`,
  `_Exemplos`, `vendor`) + reindexar sucesu e reavaliar NL.
  - **Feito (`crates/arags-embedding/src/pipeline/files.rs` + `crates/arags-cli/src/dispatch/discover.rs`):** defaults de ignore aplicados na descoberta de arquivos, casando como **segmento ou prefixo de caminho** (ex.: `vendor/foo/bar.rs`, `any/path/Seeds/x.rs` e `storage/logs/run.log` são pulados). Padrões: `Seeds`, `.seeds`, `storage/logs`, `REFERENCE`, `_Exemplos`, `vendor` (mais os já existentes `.env`, `node_modules`, `target` etc.).
  - **Configurável:** mesclados com `[project] ignore` do `.arags.toml` e com a env `ARAGS_INDEX_IGNORE` (vírgula-separada); `--force-include` ainda sobrescreve. Os defaults podem ser limpos passando lista vazia em `discover_files(root, &[], ...)`, e a função `default_index_ignores()` expõe a lista p/ extensão via config.
  - **Logging estruturado:** `tracing::info!` no fim da descoberta com `discovered` vs `ignored` e `tracing::debug!` por caminho pulado.
  - **Testes (`arags-embedding`):** `test_path_is_ignored_default_patterns`, `test_discover_files_respects_ignores`, `test_discover_files_custom_ignore_and_cleared_defaults` (custom `docs/` honrado + defaults limpos); (`arags-cli`) `test_default_ignores_noisy_corpus_paths`, `test_index_ignore_env_override`.
- `agnostic-rlm-rs-241c` (Med): **Validar llama.cpp-Vulkan na iGPU real** (Radeon 680M) — medir
  ms/chunk e confirmar ~1 ms/chunk.
- `agnostic-rlm-rs-2ff6` (Med): **Release artifact GPU** — build musl `--features llamacpp-vulkan`
  em runner com Vulkan SDK; produzir `arags-server-linux-amd64-gpu` + tag Docker `-gpu`. Não afeta
  o binário principal (candle). Relacionado a `1957`.
- `agnostic-rlm-rs-5904` (Med): **Homogeneizar prompts** de summarize (file/module/project). Criado
  `crates/arags-cli/src/prompts.rs` com `SummarizeScope { File, Module, Project }` e
  `build_summarize_prompt(scope, source, content, provenance?)` que reusa uma única instrução
  canônica ("technical writer maintaining a project knowledge base") e as mesmas seções
  obrigatórias (`## Summary`, `## Key Findings / Artifacts`, `## Related`); só a orientação de
  escopo varia. `generate_summary` (persist) e o prompt de digest (`qa_cache`) agora usam esse
  módulo. Testes: `all_scopes_contain_canonical_sections`, `scope_changes_guidance_only`,
  `provenance_optional`.
- `agnostic-rlm-rs-1119` (Med, task, **IMPLEMENTADO 2026-08-27**): **testes de integração gRPC de ponta a ponta**
  (servidor `tonic` real + cliente gerado, não só handlers isolados).
  - **Harness in-process:** `crates/arags-server/tests/grpc_integration.rs` sobe um
    `AragsServiceServer::new(AragsGrpcService::new(state))` real numa porta efêmera
    (`TcpListener::bind("127.0.0.1:0")` + `serve_with_incoming(TcpListenerStream)`), spawna
    como `tokio::task` e conecta um `AragsServiceClient::connect("http://{addr}")` de verdade.
  - **AppState somente-storage:** `AppState::with_vector_stores(storage, ServerConfig, None, None, None, None)`
    com `exploration.enabled = false` e `rlm.enabled = false` — sem vector stores e sem embedder
    de pesos (fallback), então o teste é hermético e roda offline em CI (sem Ollama/rede/modelos).
  - **Handshake de auth real:** semeia um refresh token via `tokens::create_token` e obtém a
    sessão via RPC `AuthRefresh` real; o `Bearer` é injetado no metadata de cada request.
  - **RPCs exercitados:** `auth_refresh` (handshake), `index_project` (client-streaming real:
    `Init` + `File{src/main.rs}`), e `claim_rlm_job` (unário, após semear um `rlm_jobs` pendente).
  - **Prova de persistência E2E:** após `index_project` retornar `Ok`, a MESMA `Storage` é consultada
    via novo helper `Storage::count_all_chunks()` (SQL parametrizado, `SELECT COUNT(*) FROM chunks`)
    e o total é `> 0`; `chunks_created` no `IndexResponse` é `>= 1`.
  - **Teste de desconexão sobre gRPC:** `grpc_disconnect_after_init_keeps_rlm_claim_working` —
    stream envia `Init` e TERMINA (cliente cai logo após Init); asserta `index_project` `Ok` e que um
    `claim_rlm_job` subsequente (job pendente semeado via storage) continua `Ok`/`available`,
    reproduzindo o cenário e5d0/ccc3 sobre o transporte real.
  - **Timing:** `tracing::debug!(?elapsed_ms, "grpc integration: index_project round-trip")` no teste.
  - **`tokio-stream` já era dependência** (`features = ["sync"]`); helper `count_all_chunks` adicionado
    em `crates/arags-storage/src/sqlite/chunks.rs`. `cargo clippy --workspace -D warnings`,
    `cargo fmt --check` e `cargo test -p arags-server --test grpc_integration` passam.
  - Fecha a lacuna de `agnostic-rlm-rs-b020` (cujo teste de cliente não conseguia subir servidor in-process).
 - `agnostic-rlm-rs-35a3` (Med): renomear o subcomando CLI `arags memory` → `arags maintenance` (mesmo comportamento; apenas nomenclatura). Variante de enum renomeada `Memory` → `Maintenance` em `crates/arags-cli/src/cli/commands.rs` com `#[command(name = "maintenance")]`; dispatch em `dispatch/mod.rs` atualizado. **Crate `arags-memory` (lógica de manutenção do servidor) intocado** — só o verbo do CLI e a variante do enum mudaram. Docs atualizadas: `README.md`, `crates/arags-cli/README.md`, `wiki/02-cli-arags.md`, `wiki/05-integracao-agentes.md`.
 - `agnostic-rlm-rs-50ed` (Med, **IMPLEMENTADO 2026-08-27**): **marcar falha de derivação de vetor no SQLite** para re-embed posterior (consumido por `agnostic-rlm-rs-36ae`).
   - **Reuso de `chunks.status`:** a tabela `chunks` já tem `status TEXT DEFAULT 'active'` sem CHECK, então falhas de embed/insert de chunks setam `status = 'pending_vector'` (sem nova coluna em `chunks`).
   - **Nova coluna `vector_status`:** como `rlm_nodes`, `explorations` e `qa_cache` têm `status` com CHECK restrito (não aceitam `'pending_vector'`), adicionou-se `vector_status TEXT NOT NULL DEFAULT 'indexed'` a essas três tabelas via **migração 022** (`crates/arags-storage/migrations/022_vector_status.sql`, registrada no array `MIGRATIONS` em `crates/arags-storage/src/sqlite/schema.rs`, incrementando `MIGRATION_COUNT`).
   - **Storage API (`crates/arags-storage`):** `mark_chunks_pending_vector(buffer_id, &[id])` / `chunks_pending_vector(buffer_id)` (usam `chunks.status`); `mark_rlm_nodes_pending_vector(buffer_id, &[id])` / `rlm_nodes_pending_vector(buffer_id)`; `mark_qa_cache_pending_vector(&[id])` / `qa_cache_pending_vector(project)`; `mark_explorations_pending_vector(buffer_id, &[id])` / `explorations_pending_vector(buffer_id)` (usam `vector_status`). Todos com `IN (?,?,...)` parametrizado e no-op para lista vazia.
   - **Fios de falha conectados:** `crates/arags-server/src/grpc/index.rs` (`index_stream_loop` Fase 2) marca os chunks do batch via `mark_chunks_pending_vector` em erro de `embed_batch` ou `insert_vectors` (com `warn!` + `debug!` contando linhas); `query_cache.rs` (insert da `question_vectors`), `rlm.rs` (embed/insert do espaço RLM, ambos os braços) e `exploration/mod.rs` (insert do espaço de exploração) marcam `vector_status='pending_vector'` em falha. Todos os caminhos eram `warn!`-only antes.
   - **Testes (`crates/arags-storage/tests/vector_status_test.rs`):** `vector_status_column_exists` (PRAGMA table_info confirma a coluna nas 3 tabelas), `mark_and_query_pending_vector` (+ `status='pending_vector'` e no-op de lote vazio), `mark_and_query_rlm_nodes_pending_vector`, `mark_and_query_qa_cache_pending_vector`, `mark_and_query_explorations_pending_vector` — todos verdes.
   - `cargo clippy --workspace -D warnings`, `cargo fmt --check`, `cargo test -p arags-storage` e `cargo test -p arags-server` passam.

### P3 — Integração / nice-to-have
- `agnostic-rlm-rs-9527` (Low, feature): **Integrar agente consumidor** (Tier 1: Continue/Cline/Tabby/Aider)
  ao output do arags.
- `agnostic-rlm-rs-27dc` (Backlog, epic): revisão sistêmica pós-plan 023.

---

## 3. Ordem de prioridade (roadmap resumido)

1. **P0 robustez** (`f5db`, `e5d0`, `ccc3`, `5124`) — sem isso o binário lançado pode travar/quebrar
   em disconnect ou saturar CPU sob index. *Bloqueia a confiança no release.*
2. **P1 summarizer** (`b020` + `110e`) — a feature principal do cliente ainda não validada E2E;
   CoT pollution quebraria o banco de summaries silenciosamente.
3. **P2 retrieval quality** (`6d44` A/B, `a884` ignores) — relevância em NL é o valor percebido.
4. **P2 fechar loop GPU self-contained** (`241c` bench iGPU, `2ff6` release GPU) — valida e disponibiliza
   o binário llama.cpp como artefato opcional sem tocar o release principal.
5. **P2 polimento** (`5904` prompts, `1119` testes, `35a3` rename).
6. **P3** (`9527` agente, `27dc` revisão).

---

## 4. Aprendizados — modelos (preservado, re-enquadrado)

> O summarizer é **client-side** (`arags-llm` → Ollama/OpenAI/Anthropic/Gemini). Os testes abaixo
> usaram o mesmo prompt que o cliente envia, via `/api/chat` do Ollama — sirvam para **escolher o
> modelo LOCAL do cliente**.

### 4.1 Embedding
| Modelo | Dim | Tam | Notas |
|---|---|---|---|
| `all-minilm` (atual, candle) | 384 | 23 MB | leve, rápido; **sem prefixo de task** (prefixo "search_document: " só vale p/ nomic) |
| `qwen3-embedding:0.6b` | 1024 | 596 MB | `norm=1.0`, cold ~9s; **não é chat**; candidato SOTA small-embedding p/ A/B (`6d44`) |

Como as `dims` são dinâmicas (`state.embedder.dimensions()`), trocar o modelo é sem mudança de código.

### 4.2 Summarizer (escolha do modelo local do cliente)
| Modelo | Tam | Tempo | `<think>`? | Qualidade | Veredito |
|---|---|---|---|---|---|
| `openbmb/minicpm5` | 1.1B | ~17–25s | **SIM** (sempre) | correto, c/ CoT | ❌ sem stripping |
| `llama3.2` (3B) | 3.2B | ~1.3s | não | **Bom**, estruturado | ✅ candidato |
| `qwen2.5-coder:3B` | 3.1B | n/a | n/a | n/a | ⏳ tag case (`3b`≠`3B`) |
| `qwen3:0.6b/1.7b` | 0.6/1.7B | n/a | n/a (No-Think) | n/a | ⏳ |
| `jewelzufo/ruvltra-claude-code` | 0.5B | 4.15s | não | **surpreendente** p/ 0.5B | ✅ candidato tiny |
| `granite3.1-moe:1b` | 1B (MoE) | 23s | não | ❌ autocompletou código | ❌ reprovado |
| `llama3.2:1b-instruct-q8_0` | 1B (q8_0) | 14.74s | não | **Bom**, segue instrução | ✅ candidato |
| `smollm2:360m`, `qwen2.5:0.5b`, `gemma2:2b`, `qwen2.5-coder:1.5b`, `phi3.5:mini` | — | não medido | — | — | ⏳ baixados |

**Regra dura:** modelos de **raciocínio** (MiniCPM5, Qwen3-com-think) vazam `<think>` mesmo com
`think:false` no Ollama atual → inúteis p/ summary sem stripping (**issue `110e`**).
`enable_thinking` em `options` dá 500 (só vale no transformers).

---

## 5. Referência rápida

```bash
# Build default (candle, portátil) — usado por CI/Docker/Release
cargo build --release -p arags-server

# Build GPU self-contained (OPT-IN) — exige Vulkan SDK no PATH + device em runtime
cargo build --release -p arags-server --features llamacpp-vulkan

# Benchmark llama.cpp na sua GPU
cargo run -p arags-embedding --features llamacpp-vulkan --example llamacpp_bench -- /caminho/all-minilm.gguf 99

# Docker (candle, musl estático, all-MiniLM bakeado)
docker build -f docker/Dockerfile -t arags-server .

# Usar Ollama (GPU) com o binário lançado: rode Ollama e aponte kind=ollama no server.toml
# [embedder]
# kind = "ollama"
# ollama_model = "all-minilm:22m"

# sd
sd list --status open
sd ready --format compact
```

---

## 6. Checklist de release (não-regredir)
- [ ] `cargo clippy --workspace -- -D warnings` limpo **sem** Vulkan SDK instalado (prova portabilidade).
- [ ] `docker build -f docker/Dockerfile` sem cmake/Vulkan no builder.
- [ ] `llamacpp-vulkan` continua **fora** do default (apenas opt-in) — senão quebra CI/Docker.
- [ ] `kind` default resolve para candle quando `/models` tem pesos (container OK).
