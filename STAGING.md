# STAGING.md — O que falta fazer + padrões obrigatórios

> Last updated: **2026-08-27**. Companion to the `sd` tracker (`agnostic-rlm-rs-*`).
> Architecture context: `arags-server` is a **pure data plane (no server LLM)**; summaries/digest
> happen **on the client** (`arags-cli` `query -qa` digest, `persist` summarize) using the
> **user's local LLM** via `arags-llm` (plans 019/020/023).

---

## 0. Estado atual (resumo executivo)

**Entregue e verificado** (gates verdes: `cargo clippy --workspace -- -D warnings` + `cargo fmt -- --check` + `cargo test --workspace`):

- **P0 robustez:** `e5d0` (aborto limpo no disconnect), `ccc3` (claim RLM sobrevive ao disconnect), `6690` (pool rayon capado isola CPU de index da busca), `5124` (resolvido por 6690), `20cd` (re-index sem duplicação — regressão travada).
- **P1 summarizer:** `110e` (`strip_cot` remove `<think>`), `b020` (caminho cliente E2E testável: `digest_chunks`/`generate_summary`/`write_wiki` + mock LLM).
- **P2 qualidade:** `a884` (ignores de índice), `6d44` (harness A/B de relevância), `5904` (prompts homogeneizados), `35a3` (`arags memory`→`arags maintenance`), `1119` (integração gRPC in-process).
- **Fundação dos epics server (desbloqueadores):** `b1a0` (migração 021: colunas temporais + índices parciais em 4 tabelas), `50ed` (`pending_vector` marcado em falha de embed em todos os 4 espaços).
- **Entregue nesta sessão (orquestração 2026-08-27, 2ª rodada):** `1564` (time-travel completo nos 4 espaços) — gates verdes (clippy/fmt/test) + testes dedicados.
**Entregue nesta sessão (orquestração 2026-08-27):** `8dcc` (chunks imutáveis/supersede), `786a` (autoria created_by/model), `e210` (superseding de derivados) — todos com gates verdes (clippy/fmt/test) e testes dedicados; `1564` (time-travel) **entregue na 2ª rodada** (gates verdes, ver seção 7). **O que FALTA (seção 2):** 22 issues em aberto (4 fechadas) — consistência temporal/vetorial (`36ae`/`620d`/`c7b1`/`49d6`), quorum/segurança RLM (`a5d7`/`6d97`/`64af`/`f486`/`f5f3`/`e89e`/`d172`), CLI/UX (`e5d8`/`7aa8`), GPU/build/CI (`241c`/`2ff6`/`1957`/`d607`/`0fc4`), e integração/revisão (`9527`/`e9e3`/`27dc`/`7222`). Cada um detalhado com os padrões da seção 1.

---

## 1. Padrões gerais obrigatórios (NÃO REGREDIR)

Toda issue nova/continuada DEVE obedecer isto. Violações bloqueiam o fechamento.

### 1.1 Toolchain / linguagem
- **Rust Edition 2024**, `rust-version = "1.85"`.
- `cargo clippy --workspace -- -D warnings` **deve passar** (`clippy::pedantic = "warn"`, `clippy::unwrap_used = "deny"`, `clippy::expect_used = "deny"`, `clippy::panic = "deny"`, `unsafe_code = "forbid"` salvo bloco `unsafe` justificado com `// SAFETY:`).
- `cargo fmt` + `cargo fmt -- --check` **limpo**.

### 1.2 Tratamento de erro
- **Nunca** `unwrap()`/`expect()`/`panic!()`/`unsafe` em código de produção. Use `?`, `anyhow` (app), `thiserror` (lib).
- Em `#[cfg(test)]`: permita com `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` **no topo do módulo de teste** (não espalhe `#[allow]` em produção).

### 1.3 Testes (obrigatórios)
- Cada função pública tem ≥ 1 teste. Use `tempfile` para FS/DB.
- `proptest` para parsing/chunking/scoring; mocks via trait objects (sem rede em unit test).
- Handler tests podem usar `ReceiverStream`/`TcpListenerStream` efêmero (padrão `crates/arags-server/tests/grpc_integration.rs`).
- Regressão E2E com servidor in-process quando o handler mexer em transporte/conn.
- Verificação antes de fechar: `cargo test --workspace`.

### 1.4 Logs estruturados + timing (mapeamento de tempo por função)
- Projeto usa **`tracing` + `tracing-subscriber` (feature `json`)**. Nunca `println!` para log em produção.
- Handlers/funções caras recebem `#[instrument(skip_all, fields(buffer_id, project, ...))]`; popule os fields quando conhecidos.
- Emita `debug!`/`info!` com `phase` + `elapsed_ms` (ex.: `phase = "phase2_embed_batch", elapsed_ms = t0.elapsed().as_millis() as u64`). Isso mapeia o tempo de execução de cada função/fase.
- Conexões/transações SQLite ficam **escopadas dentro de `store::blocking(...)`** — nada retido entre iterações/`.await` (garante que disconnect não vaza pool, validado em `e5d0`/`ccc3`).

### 1.5 SQL / storage
- **SQL 100% parametrizado** (`?1`, `?2` ou named). Nunca interpole valores. `IN (...)` usa placeholders construídos com segurança.
- FTS5: sempre passe queries por `arags-storage/src/fts.rs::sanitize_fts` antes de `MATCH`.
- **Migrations:** `crates/arags-storage/migrations/NNN_*.sql`, registrados no array `MIGRATIONS` (`include_str!`) em `crates/arags-storage/src/sqlite/schema.rs` (incrementa `MIGRATION_COUNT`). Colunas simples → `ALTER TABLE ... ADD COLUMN ... DEFAULT ...`. Mudar CHECK → padrão `_new` + copy + rename (ver `020_add_exploration_review.sql`). Idempotente (runner skipa via `schema_version`).

### 1.6 Convenções rAGS (resumo)
- `clippy` deny em produção; `anyhow`/`thiserror`; imports explícitos (sem `use foo::*`); `Cow<'a, str>`/`Arc<str>` para zero-copy; `AtomicU*` para contadores; `parking_lot` para locks; Rayon/`spawn_blocking` para CPU/DB-bound; embed confinado ao `index_embed_pool` capado (ver `6690`).
- `mimalloc` como allocator global.

### 1.7 Rastreamento `sd` + STAGING
- Ao começar: `sd update <id> --status in_progress` → `sd sync`.
- Ao terminar: `sd close <id> --reason "..."` → `sd sync`. (sd em `/home/one/.bun/bin/sd`.)
- Atualizar a seção 2 deste `STAGING.md` (bullet da issue) com o que foi feito + nomes de testes.
- Orquestração: **1 issue = 1 subagente** (`Task` tool, `subagent_type = "general"`), cadeia em ordem de dependência, verificar gates antes de fechar.

---

## 2. O que falta — 12 issues em aberto (ordenadas por cluster)

> `⛔` = bloqueada por outra issue (ver `sd show`). `f5db` está `IN PROGRESS` (CLI pronta; partes server nas issues abaixo) e não entra nesta lista de "open".

### Cluster A — Consistência temporal / vetorial (storage & evolução do conhecimento)

- **`agnostic-rlm-rs-49d6`** ✅ **DONE** (Critical, epic) — *Consistência SQLite↔vetores*: fonte canônica SQLite com reconcile worker `36ae`, bootstrap/rebuild `620d`, e re-digest QA por fila com lease `d172`. Concluído.
- **`agnostic-rlm-rs-8dcc`** ✅ **DONE** (High, task) — *Chunks imutáveis*: re-index insere nova versão e **supersede** a antiga (soft-delete `is_active=0` + drop de FTS row + vetor usearch; leituras/FTS/busca não retornam inativos). Retire de órfãos no fim do stream; `purge_inactive_chunks` no ticker com `chunk_retention_days` (migração `023_inactive_retention.sql`). Testes: `reindex_supersedes_old_chunk_history_retained`, `purge_inactive_chunks_respects_retention_window`, `retire_chunk_drops_fts_marks_inactive_links_superseder`; regressão `reindex_replaces_chunks_without_duplication` verde. Padrões §1.4/§1.5/§1.3.
- **`agnostic-rlm-rs-36ae`** ✅ **DONE** (High, task) — *Reconcile worker*: novo `crates/arags-server/src/reconcile.rs` (`reconcile_pending_vectors(state)`) varre `pending_vector` (marcado em `50ed`) nos 4 espaços e re-embutir do conteúdo canônico no pool `index_embed` capado, reinserindo nos espaços usearch; limpa flag `pending_vector`; métricas/gap logs (`pending`/`processed`/`remaining` + `elapsed_ms`). Helpers `get_*_embed_inputs`/`clear_*_pending_vector` no storage. Integrado ao ticker de `maintenance` em `lifecycle.rs`. Sem nova migration. Testes: `reconcile_clears_pending_vector_and_inserts_chunk_vector`, `reconcile_handles_all_four_spaces_gap_metrics`, `reconcile_remarks_pending_on_embed_failure`. Padrões §1.4/§1.6/§1.3.
- **`agnostic-rlm-rs-620d`** ✅ **DONE** (High, task) — *Bootstrap/rebuild*: `crates/arags-server/src/bootstrap.rs` reconstrói os 4 espaços a partir do SQLite quando divergentes (compara contagens, re-embed em lote no pool capado, persiste); em sincronia é pulado. `clear()` em `VectorSpaceStore`+4 stores; flush documentado como otimização. Helpers `all_*_embed_inputs`. Sem nova migration. Testes: `bootstrap_rebuilds_divergent_chunk_space_from_sqlite`, `bootstrap_skips_in_sync_space`, `bootstrap_rebuilds_all_four_spaces_when_empty`. Padrões §1.4/§1.6/§1.3.
- **`agnostic-rlm-rs-c7b1`** ✅ **DONE** (High, epic) — *Evolução temporal*: epochs/soft-versioning + metadados de autoria nas 4 frentes. Entregue via sub-steps `b1a0`+`8dcc`+`786a`+`e210`+`1564` (plano `pl-3fc2` concluído).
- **`agnostic-rlm-rs-786a`** ✅ **DONE** (Medium, task) — *Propagar autoria*: `created_by` (username da sessão autenticada) + `model` preenchidos em toda escrita server-side (chunks via `index_stream_loop`/`insert_chunks_batched` com `state.embedder.name()`, QA `store_answer`, RLM `complete`, explorations). Testes: `insert_chunks_batched_populates_created_by_and_model`, `store_answer_populates_created_by`, `complete_with_node_persists_created_by_and_model`. Padrões §1.2/§1.5/§1.4.
- **`agnostic-rlm-rs-e210`** ✅ **DONE** (Medium, feature) — *Superseding de derivados*: nova resposta/nó/mapa = novo registro que supersede o anterior (`is_active=0` + `superseded_by`); leituras filtram `is_active=1`; getters históricos (`get_answer_history`/`get_node_history`/`get_exploration_history`) seguem a cadeia. Migração `024_supersede_derived.sql` (índices únicos parciais por subject ativo). Testes: `supersede_*_creates_new_active_row_and_history` (qa_cache/rlm/explorations). Padrões §1.5/§1.4.
- **`agnostic-rlm-rs-1564`** ✅ **DONE** (Low, feature) — *Time-travel*: `as_of_epoch`/`as_of_timestamp` (proto: search/query_cache/exploration) + getters `*_as_of` nos 4 espaços (chunks `get_chunk_as_of`, qa_cache `get_cached_answer_as_of`, rlm `get_rlm_node_as_of`, explorations `get_exploration_as_of`/`_by_id`) retornando a revisão ativa naquele epoch (cadeia superseded_by/`epoch <= as_of`). Handlers aplicam filtro as-of; CLI `--as-of-epoch`/`--as-of` com `resolve_as_of_epoch`; renderização marca snapshot time-travel em text/jsonl/full_json. Sem nova migration (epoch/created_by/model já em 021/024). Testes: `time_travel_search_returns_version_active_at_epoch`, `time_travel_query_returns_superseded_answer_as_of`, `time_travel_rlm_summary_as_of`, `time_travel_exploration_as_of` (arquivo `crates/arags-storage/tests/temporal_as_of_test.rs`). Padrões §1.5/§1.4.

### Cluster B — RLM: quorum / segurança / multi-voluntário

- **`agnostic-rlm-rs-a5d7`** ✅ **DONE** (High, task) — *Design + config de quorum*: `QuorumConfig` em `config.rs` (n=3, `quorum_sim_threshold=0.85`, `FusionStrategy` enum, `strikes_limit=3`) com defaults; migration `026_submissions` (tabela `submissions` candidate/accepted/rejected + índices) + `volunteer_trust`; módulo `submissions.rs` (insert/accept/reject/list_pending/record_strike). Decisão cosseno/fusão deixada para `6d97`/`64af`. Testes: `quorum_config_defaults_when_section_absent`, `submissions_insert_and_transition_candidate_to_accepted`, `submissions_reject_records_strike`, `submissions_list_pending_scoped_by_subject`. Padrões §1.5/§1.4.
- **`agnostic-rlm-rs-6d97`** ✅ **DONE** (High, feature) — *Multi-atribuição RLM + quorum cosseno*: fan-out de job RLM em N slots independentes (`generation_group_id`, migration `027_rlm_generation_group.sql`) com lease por voluntário; `CompleteRlmJob` estagia candidate em `submissions` e dispara `decide_rlm_quorum` (`crates/arags-server/src/quorum.rs`) — embute, cosseno par-a-par, claque ≥ `quorum_sim_threshold`, fusão por `FusionStrategy`, publica nó + accept/reject + `record_strike`. Idempotente; n==1 mantém path legado. Testes: `rlm_job_creates_n_independent_slots`, `rlm_quorum_accepts_fused_consensus`, `rlm_quorum_rejects_when_all_diverge`, `rlm_quorum_pending_until_n_candidates`, `rlm_quorum_is_idempotent_after_accept`. Padrões §1.5/§1.4.
- **`agnostic-rlm-rs-64af`** ✅ **DONE** (High, epic) — *Quorum matemático (BFT leve)*: atestado HMAC/session-binding (`sign_rlm_submission` em `arags-core::rlm_attestation`; verificação em `grpc/rlm.rs` `phase=rlm_submission_verify` com `subtle` constant-time; `submission_hmac` no proto); bound byzantino `f = floor((n-1)/3)` exige `>= 2f+1` concordantes em `decide_rlm_quorum`; fusão ponderada por `trust_score` (`f486`). Deps `hmac`/`subtle`. Testes: `rlm_submission_valid_hmac_accepted`, `rlm_submission_invalid_hmac_rejected`, `rlm_quorum_requires_2f_plus_1`, `rlm_quorum_fusion_prefers_higher_trust`. Padrões §1.5/§1.4.
- **`agnostic-rlm-rs-f486`** ✅ **DONE** (Medium, feature) — *Trust score de voluntário*: `record_strike` decai `trust_score` (−0.2) e retorna `(strikes, trust_score)`; `bump_trust_on_accept` (+0.1, perdoa strike); `is_banned`; `list_volunteers_by_trust` (ranking). `claim_rlm_job` rejeita banidos + exclui divergers (`rlm_job_exclusions`, migration `028_rlm_exclusions.sql`). `quorum.rs` reatribui após divergência total (nova geração excluindo divergers, cap `strikes_limit` rounds, log `phase=rlm_quorum_reassign`). Testes: `trust_score_decreases_on_strike_and_increases_on_accept`, `list_volunteers_by_trust_ranks_correctly`, `banned_volunteer_claim_is_rejected`, `diverger_is_excluded_from_reassigned_generation_group`, `total_divergence_triggers_reassignment_excluding_divergers`, `total_divergence_is_capped_after_strikes_limit_rounds`. Padrões §1.5/§1.4.
- **`agnostic-rlm-rs-f5f3`** ✅ **DONE** (High, feature) — *Remover `explore feedback` da superfície pública*: RPC `FeedbackExploration` + `FeedbackExplorationRequest`/`Response` + `FeedbackKind` removidos do proto; handler público deletado (mantidos admin `invalidate`/`review`); CLI `arags explore feedback` removido; `tests_feedback.rs` substituído por `tests_moderation.rs` (2 testes preservados). Aprovação de non-admin já é via quorum/`submissions` (`e89e`). Doctest `compile_fail` `exploration_public_feedback_surface_removed` prova a remoção. DB columns de feedback preservadas (não escritas). Padrões §1.5/§1.4.
- **`agnostic-rlm-rs-e89e`** ✅ **DONE** (Medium, feature) — *Explorations non-admin*: `ValidationMode` enum (Quorum default | Review) em `ExplorationConfig`; persist roteia — admin auto-aprova; non-admin Review+require_review→`pending_review`; non-admin Quorum→mapa `pending_review` (não surfacado) + candidate em `submissions`. Testes: `exploration_validation_mode_defaults_to_quorum`, `exploration_admin_auto_approves_in_quorum_mode`, `exploration_nonadmin_quorum_creates_submission_candidate`, `exploration_nonadmin_review_mode_goes_to_pending_review`. Decisão cosseno deixada para `6d97`/`64af`. Padrões §1.5/§1.4.
- **`agnostic-rlm-rs-d172`** ✅ **DONE** (Medium, feature) — *Re-digest de QA via fila com lease*: migration `025_pending_qa` + módulo `pending_qa.rs` (`enqueue_pending_qa` idempotente, `claim_pending_qa` prefere `preferred_user`, `revert_expired_leases` 300s, `complete_pending_qa`); `mark_qa_stale` auto-enfileira; `reclaim_expired_pending_qa` no ticker; proto `ClaimPendingQa`/`CompletePendingQa` + handlers. Testes: `enqueue_pending_qa_is_idempotent`, `claim_pending_qa_prefers_preferred_user`, `claim_pending_qa_lease_expires_and_requeues`, `pending_qa_lifecycle_claim_store_complete`, `enqueue_pending_qa_for_stale_autofills_author`. Padrões §1.5/§1.4.

### Cluster C — CLI / UX / superfície

- **`agnostic-rlm-rs-e5d8`** (High, feature) — *`arags init` completo*: wizard interativo (TTY) ou flags para todo o reconhecimento do projeto (estende o `init --name` de `f5db`). Padrões §1.3/§1.4.
- **`agnostic-rlm-rs-7aa8`** (Medium, task) — *Repensar superfície*: `arags search` = busca objetiva (sem question); `arags query` → `arags ask` com `-qa` implícito. Padrões §1.3.

### Cluster D — GPU / build / CI

- **`agnostic-rlm-rs-241c`** (Medium, task) — *Validar llama.cpp-Vulkan na iGPU real* (Radeon 680M): medir ms/chunk (~1 ms esperado). Exige hardware. Padrões §1.1/§1.4.
- **`agnostic-rlm-rs-2ff6`** (Medium, task) — *Release artifact GPU*: build musl `--features llamacpp-vulkan` + tag Docker `-gpu`. Exige runner com Vulkan SDK. Padrões §1.1/§6.
- **`agnostic-rlm-rs-1957`** (Medium, task) — *CI/CD matriz de targets* (Debian/musl/AlmaLinux/Windows Server), mac ARM-only, wiring `ARAGS_BIN_URL` no Docker. Padrões §1.1.
- **`agnostic-rlm-rs-d607`** (Low, task) — *CI/release com baseline x86-64-v2*; `target-cpu=native` apenas local. Padrões §1.1.
- **`agnostic-rlm-rs-0fc4`** (Medium, task) — *021.9: completar splits dos 9 arquivos em allowlist* (gate de linhas). Padrões §1.3.

### Cluster E — Integração / revisão / misc

- **`agnostic-rlm-rs-9527`** (Low, feature) — *Integrar agente consumidor* (Tier 1: Continue/Cline/Tabby/Aider) ao output do arags. Padrões §1.3.
- **`agnostic-rlm-rs-e9e3`** (Low, bug) — *VERIFICAR*: `explore search` retorna "no exploration maps" após persist (vetor de exploração ausente na busca semântica). Padrões §1.5/§1.3.
- **`agnostic-rlm-rs-27dc`** (Backlog, epic) — *Revisão sistêmica pós-plan 023* (adiada). Pode fechar como documentação.
- **`agnostic-rlm-rs-7222`** (Backlog, feature) — *Multi-user roadmap reescopado*: rate-limiting + audit log (auth já coberto pelo plan 018).

---

## 3. Ordem de prioridade (roadmap)

1. **Cluster A** (`8dcc`→`786a`→`e210`→`1564`→`36ae`/`620d`→`c7b1`, +`49d6`) — conhecimento temporal; base para fechar `f5db`. *Maior blast radius: toca todos os caminhos de leitura (filtrar `is_active`).*
2. **Cluster B** (quorum RLM: `a5d7`→`6d97`/`64af`/`f486`/`f5f3`/`e89e`/`d172`) — segurança/consenso; vários bloqueados.
3. **Cluster C** (`e5d8`, `7aa8`) — CLI/UX.
4. **Cluster D** (`241c`/`2ff6`/`1957`/`d607`/`0fc4`) — GPU/CI; exige hardware/runner.
5. **Cluster E** (`9527`/`e9e3`/`27dc`/`7222`) — integração/revisão.

> **Decisão pendente do orquestrador (não automatizar sem confirmação):** o Cluster A é um refactor arquitetural grande. O `8dcc` muda a semântica de re-index (supersede vs delete+reinsert verificado em `20cd`). Antes de prosseguir além deste ponto, confirme se segue autonomamente o Cluster A ou apenas um subconjunto (`8dcc`+`36ae`, ou só `786a`).

---

## 4. Aprendizados — modelos (preservado)

> O summarizer é **client-side** (`arags-llm` → Ollama/OpenAI/Anthropic/Gemini).

### 4.1 Embedding
| Modelo | Dim | Tam | Notas |
|---|---|---|---|
| `all-minilm` (atual, candle) | 384 | 23 MB | leve, rápido; **sem prefixo de task** |
| `qwen3-embedding:0.6b` | 1024 | 596 MB | `norm=1.0`, cold ~9s; **não é chat**; candidato SOTA p/ A/B (`6d44`, bake deferido) |

`dims` dinâmicas (`state.embedder.dimensions()`) → trocar modelo sem mudança de código.

### 4.2 Summarizer (modelo local do cliente)
| Modelo | Tam | Tempo | `<think>`? | Veredito |
|---|---|---|---|---|
| `llama3.2` (3B) | 3.2B | ~1.3s | não | ✅ candidato |
| `jewelzufo/ruvltra-claude-code` | 0.5B | 4.15s | não | ✅ candidato tiny |
| `llama3.2:1b-instruct-q8_0` | 1B | 14.74s | não | ✅ candidato |
| `openbmb/minicpm5` | 1.1B | ~17–25s | **SIM** | ❌ sem `strip_cot` (`110e` cobre) |
| `granite3.1-moe:1b` | 1B | 23s | não | ❌ autocompletou código |

**Regra dura:** modelos de raciocínio vazam `<think>` mesmo com `think:false` → inúteis sem `strip_cot` (`110e`). `strip_cot` já aplicado em `digest_chunks`/`generate_summary` e preserva markdown/quebras.

---

## 5. Referência rápida

```bash
# Gates obrigatórios antes de fechar qualquer issue
cargo clippy --workspace -- -D warnings
cargo fmt -- --check
cargo test --workspace

# Build default (candle, portátil) — CI/Docker/Release
cargo build --release -p arags-server

# Build GPU self-contained (OPT-IN, exige Vulkan SDK)
cargo build --release -p arags-server --features llamacpp-vulkan

# Integração gRPC in-process (padrão 1119)
cargo test -p arags-server --test grpc_integration

# sd
sd list --status open
sd ready --format compact
```

---

## 6. Checklist de release (não-regredir)
- [ ] `cargo clippy --workspace -- -D warnings` limpo **sem** Vulkan SDK (prova portabilidade).
- [ ] `docker build -f docker/Dockerfile` sem cmake/Vulkan no builder.
- [ ] `llamacpp-vulkan` continua **fora** do default (apenas opt-in).
- [ ] `kind` default resolve para candle quando `/models` tem pesos.
- [ ] Todo handler de escrita loga `elapsed_ms` + `phase` (§1.4).
- [ ] Todo SQL parametrizado; FTS5 sanitizado (§1.5).

---

## 7. Log de orquestração (sessão 2026-08-27)

Orquestrador executou **1 subagente por issue** (padrão §1.7), na ordem do plano `pl-3fc2`. Cada subagente recebeu o conhecimento do codebase (arquivos/阅读和 padrões §1) e os gates foram verificados independentemente pelo orquestrador (clippy/fmt + testes nomeados) antes do fechamento no `sd`.

| Issue | Status seed | Subagente | Gates | Testes-chave adicionados/verificados |
|---|---|---|---|---|
| `8dcc` | ✅ closed | sim (general) | clippy/fmt/test verdes | `reindex_supersedes_old_chunk_history_retained`, `purge_inactive_chunks_respects_retention_window`, `retire_chunk_drops_fts_marks_inactive_links_superseder`; regressão `reindex_replaces_chunks_without_duplication` mantida |
| `786a` | ✅ closed | sim (general) | clippy/fmt/test verdes | `insert_chunks_batched_populates_created_by_and_model`, `store_answer_populates_created_by`, `complete_with_node_persists_created_by_and_model` |
| `e210` | ✅ closed | sim (general) | clippy/fmt/test verdes | `supersede_qa_creates_new_active_row_and_history`, `supersede_rlm_node_creates_new_active_row_and_history`, `supersede_exploration_creates_new_active_row_and_history` |
| `1564` | ✅ closed | sim (general) | clippy/fmt/test verdes | `time_travel_search_returns_version_active_at_epoch`, `time_travel_query_returns_superseded_answer_as_of`, `time_travel_rlm_summary_as_of`, `time_travel_exploration_as_of` (arquivo `crates/arags-storage/tests/temporal_as_of_test.rs`) |

**2ª rodada de orquestração (2026-08-27) — usuário autorizou "prossiga conforme fila".** 1 subagente `general` por issue (§1.7); gates verificados independentemente pelo orquestrador (fmt/clippy/test) antes de cada fechamento.

| Issue | Status seed | Subagente | Gates | Testes-chave adicionados/verificados |
|---|---|---|---|---|
| `36ae` | ✅ closed | sim (general) | clippy/fmt/test verdes | `reconcile_clears_pending_vector_and_inserts_chunk_vector`, `reconcile_handles_all_four_spaces_gap_metrics`, `reconcile_remarks_pending_on_embed_failure` |
| `620d` | ✅ closed | sim (general) | clippy/fmt/test verdes | `bootstrap_rebuilds_divergent_chunk_space_from_sqlite`, `bootstrap_skips_in_sync_space`, `bootstrap_rebuilds_all_four_spaces_when_empty` |
| `c7b1` | ✅ closed | — (epic) | gates verdes | plano `pl-3fc2` concluído (sub-steps 8dcc/786a/e210/1564 já fechados/testados) |
| `d172` | ✅ closed | sim (general) | clippy/fmt/test verdes | `enqueue_pending_qa_is_idempotent`, `claim_pending_qa_prefers_preferred_user`, `claim_pending_qa_lease_expires_and_requeues`, `pending_qa_lifecycle_claim_store_complete`, `enqueue_pending_qa_for_stale_autofills_author` |
| `49d6` | ✅ closed | — (epic) | gates verdes | consistência entregue via 50ed+36ae+620d+d172 |
| `a5d7` | ✅ closed | sim (general) | clippy/fmt/test verdes | `quorum_config_defaults_when_section_absent`, `submissions_insert_and_transition_candidate_to_accepted`, `submissions_reject_records_strike`, `submissions_list_pending_scoped_by_subject` |
| `e89e` | ✅ closed | sim (general) | clippy/fmt/test verdes | `exploration_validation_mode_defaults_to_quorum`, `exploration_admin_auto_approves_in_quorum_mode`, `exploration_nonadmin_quorum_creates_submission_candidate`, `exploration_nonadmin_review_mode_goes_to_pending_review` |

| `6d97` | ✅ closed | sim (general) | clippy/fmt/test verdes | `rlm_job_creates_n_independent_slots`, `rlm_quorum_accepts_fused_consensus`, `rlm_quorum_rejects_when_all_diverge`, `rlm_quorum_pending_until_n_candidates`, `rlm_quorum_is_idempotent_after_accept` |

**Recuperação pós-incidente (2026-08-27, janela 10:31–11:02 apagada antes do commit):** as 3 issues restantes do Cluster B foram reimplementadas fielmente a partir do conversation recuperado em `CRITICAL_RECUPERATION/`, 1 subagente por issue (§1.7), gates verificados independentemente pelo orquestrador (`cargo clippy --workspace -- -D warnings` + `cargo fmt -- --check` + `cargo test --workspace` ok).

| Issue | Status seed | Subagente | Gates | Testes-chave adicionados/verificados |
|---|---|---|---|---|
| `f5f3` | ✅ closed | sim (general) | clippy/fmt/test verdes | `exploration_public_feedback_surface_removed` (doctest `compile_fail`); `invalidate_requires_admin_and_modes_behave`, `review_gate_holds_non_admin_maps_until_approved` preservados; grep prova remoção da superfície |
| `f486` | ✅ closed | sim (general) | clippy/fmt/test verdes | `trust_score_decreases_on_strike_and_increases_on_accept`, `list_volunteers_by_trust_ranks_correctly`, `banned_volunteer_claim_is_rejected`, `diverger_is_excluded_from_reassigned_generation_group`, `total_divergence_triggers_reassignment_excluding_divergers`, `total_divergence_is_capped_after_strikes_limit_rounds` |
| `64af` | ✅ closed | sim (general) | clippy/fmt/test verdes | `rlm_submission_valid_hmac_accepted`, `rlm_submission_invalid_hmac_rejected`, `rlm_quorum_requires_2f_plus_1`, `rlm_quorum_fusion_prefers_higher_trust` |

**Migrations novas (pós-incidente):** `028_rlm_exclusions.sql` (f486); nenhuma nova migration para f5f3/64af (campos de proto + tabela `rlm_job_exclusions` via 028).

**Estado ao checkpoint (pós-recuperação):** Cluster A 100% concluído. Cluster B 100% concluído (`a5d7`+`e89e`+`6d97`+`f486`+`f5f3`+`64af`). Próximo da fila: Cluster C (`e5d8`,`7aa8`), D (GPU/CI, exige hardware), E (`9527`/`e9e3`/`27dc`/`7222`). Manter verificação independente do orquestrador antes de fechar cada seed.

**Migrations novas (2ª rodada):** `025_pending_qa.sql` (d172), `026_submissions.sql` (a5d7). Nenhuma para 36ae/620d/1564 (colunas já em 021/024/050ed).

**Estado ao checkpoint:** Cluster A 100% concluído (8dcc/786a/e210/1564/36ae/620d/c7b1 + epic 49d6 + d172). Cluster B: `a5d7` (keystone) + `e89e` fechados — candidatos já são estagiados em `submissions`. **Restam em B (pesados, decisão cosseno):** `6d97` (multi-atribuição + quorum cosseno/fusão), `64af` (BFT leve), `f486` (trust score), `f5f3` (remover feedback público). `6d97`/`64af`/`f486` agora desbloqueados (dependiam de `a5d7`/`e89e`). Próximo da fila: `6d97`. Manter verificação independente do orquestrador antes de fechar cada seed.
