# Plan 019: Remoção do Legado RLM e Consolidação da CLI

## Context

Os planos 017 (QA-Cache, digestão no client) e 018 (auth por refresh token) estabeleceram a
filosofia **on-demand, agent-agnostic, sem LLM no servidor**. O projeto acumulou, porém, um
"legado RLM recursivo" — o orquestrador (`arags-core/src/engine/*`, `planner`, `solver`,
`synthesizer`, `repl`), o `arags run`, o servidor de RLM (`grpc/runs`), o `summarizer` hierárquico
e a config `[llm]` do server — que não faz mais sentido:

- `arags run` dispara um loop recursivo `planner → solver → synthesizer` que **não existe mais** na
  arquitetura sob demanda. A filosofia agora é "processamento sob demanda via `-qa`".
- O `summarizer` (server-side LLM) duplica o LLM que o agente consumidor já possui e contradiz o
  plano 017 ("servidor sem LLM").
- A config `[llm]` do server só era usada por `run` + `summarize` (`grpc/runs/engine.rs`,
  `summarizer/engine.rs`) — ambos removidos.
- Comandos derivados do run (`status [run_id]`, `history` de runs, `cost`, `cancel`, `checkpoints`)
  ficam órfãos.

Paralelamente, a superfície de CLI cresceu desordenada. Este plano **consolida a CLI** em um
conjunto enxuto e coerente (init/index/search/query/memory/persist/history), **remove o legado
RLM** de ponta a ponta (core → server → proto → cli → storage), e **repensa** os comandos
`persist` e `history` e a gestão de memória (`cache`+`decay`+`entities` → `arags memory` admin).

Este plano **não implementa o 017/018** (já feitos); ele **age em cima deles**: remove o que
ficou obsoleto e unifica a superfície.

---

## Goals

- **Remover por completo** o legado RLM: engine (`arags-core`), `arags run` (CLI), `grpc/runs`
  (server), `summarizer`, config `[llm]` do server, RPCs `Run`/`Summarize`, e as tabelas
  `runs`/`run_model_usage`/`trajectories`/`sessions`(multi-turn)/`session_*`/`checkpoints`.
- **CLI enxuta e coerente** com on-demand: `init`, `index`, `search`, `query`, `memory` (admin),
  `persist` (rework), `history` (por usuário).
- **Unificar** `cache`+`decay`+`entities` em `arags memory` (admin, role do plan 018).
- **Repensar `persist`**: exibir `response_id`, gerar `wiki/yyyymmddhhmm_title.md` estruturado
  usando resumo via IA real (modelo do usuário, `--qa`).
- **`history` por usuário**, atrelado ao refresh token (server-side, escopado por `username`).
- **Novo `arags init`**: prepara o repo (config local + `arags index`).
- Servidor **sem LLM** e sem manutenção manual de memória no client: `consolidate`/`decay` viram
  **manutenção server-side** (cron + RPC admin).

## Non-goals

- Não reintroduz LLM no servidor.
- Não cria IdP/UI — auth segue plan 018.
- Não mexê na busca híbrida nem no QA-Cache (plan 017) em si; só na forma como são expostos.
- Não remove `auth_tokens`/`auth_sessions` (plan 018) nem o `qa_cache` (plan 017).
- `wiki/` permanece como **pasta de saída** do `persist` (sem o comando `wiki` git).

---

## Superfície de CLI alvo

| Comando | Finalidade | Estado |
|---|---|---|
| `arags init [--index]` | prepara repo (config `.arags/` + roda `index`) | **NOVO** |
| `arags index` | ingestão (chunking + embed + store) | mantido |
| `arags search` | busca híbrida (BM25+vetor+entity), sem LLM | mantido |
| `arags query` | on-demand QA: `-qa` digest no client; `--cache-id` lookup determinístico | mantido (refinado) |
| `arags memory` (admin) | unifica `cache`+`decay`+`entities`: `list`/`get`/`invalidate`/`cleanup` | **NOVO** (subs. cache/decay/entities) |
| `arags persist <response_id>` | de `response_id` → `wiki/yyyymmddhhmm_title.md` estruturado | **REWORK** |
| `arags history [--limit]` | histórico de queries do usuário (server, por refresh token) | **REWORK** (por-usuário) |
| `arags run` | — | **REMOVIDO** |
| `arags context` | — | **REMOVIDO** (redundante c/ `query` sem `--llm`) |
| `arags session` | — | **REMOVIDO** (justificado abaixo) |
| `arags status` | — | **REMOVIDO** |
| `arags cost` | — | **REMOVIDO** |
| `arags cancel` | — | **REMOVIDO** |
| `arags checkpoints` | — | **REMOVIDO** |
| `arags restore-page` | — | **REMOVIDO** |
| `arags wiki` | — | **REMOVIDO** (cmd; dir `wiki/` mantido p/ `persist`) |
| `arags consolidate` (CLI) | — | **REMOVIDO do CLI** → manutenção server-side |
| `arags decay` (CLI) | — | **REMOVIDO do CLI** → `memory cleanup` |

**`arags server`** (gRPC/MCP) mantém os mesmos endpoints de negócio (index/search/query/context/
memory/history) e **perde** o endpoint `/run`. O `arags-server admin` (CLI interno do container)
ganha `consolidate`.

---

## Architecture Overview

```
 CLIENT (arags-cli)                              SERVER (arags-server, sem LLM)
 ┌────────────────────────────┐                ┌──────────────────────────────────────┐
 │ init   → scaffold .arags/ + index            │ gRPC business:                        │
 │ index  → ingestão local → gRPC IndexProject │   IndexProject, Search, QueryWithCache │
 │ search → gRPC Search                        │   StoreAnswer, GetAnswerById          │
 │ query  → -qa digest no client (arags-llm)    │   InvalidateCache (admin)             │
 │         --cache-id lookup 1:1               │   ListMemory / GetCache (admin)       │
 │ memory (admin) → List/Get/Invalidate/Cleanup│   TriggerMaintenance (admin)          │
 │ persist <id> → resumo via arags-llm →        │   GetHistory {user} (por usuário)     │
 │   wiki/yyyymmddhhmm_title.md                │                                        │
 │ history → GetHistory(user)                  │ Maintenance (interno, sem gRPC):       │
 │                                            │   consolidate() + decay() a cada tick │
 │  auth_client: AuthRefresh → bearer          │   (cron via [maintenance] interval)   │
 └────────────────────────────┘                └──────────────────────────────────────┘
```

---

## A. Remoção do legado RLM (core → server → proto → cli → storage)

### A.1 `arags-core` (remover módulos engine)
Remover arquivos: `engine/`, `planner.rs`, `solver.rs`, `synthesizer.rs`, `repl.rs`,
`budget.rs`, `cache.rs`, `tools.rs`, `router.rs`, `sampling.rs`, `guardrails.rs`,
`compaction.rs`, `docker.rs`, `jsonl_logger.rs`, `token_counter.rs`, `concurrency.rs`,
`events.rs`.
Manter: `types` (podado), `logging`, `qa_cache`, `memory` (só a trait `MemoryProvider`).

Ajustes:
- `src/lib.rs`: remover re-exports (`run_rlm_engine*`, `StartRunInput`, `RlmBackend`, etc.).
- `src/types/mod.rs`: remover `pub use crate::tools::{...}` (linha 21) e podar `enums.rs`
  (`RlmBackend`, `RunOutput`, `CompactionPolicy`…), `input.rs` (`StartRunInput`),
  `node.rs` (`RlmNode`) — manter apenas tipos usados por `qa_cache`/`memory` sobreviventes.
- `src/memory/mod.rs`: manter a trait `MemoryProvider` (consumida por `arags-memory`).

**Acoplamento crítico (`arags-memory`):** `crates/arags-memory/src/engine/memory_api.rs`
importa `arags_core::types::{RlmNode, RlmRunResult, StartRunInput}`. Esses tipos somem →
`memory_api.rs` fica órfão. Resolução: remover `memory_api.rs` e os módulos de memory **atrelados
a run** (`trajectory/`, `checkpoint/`, `session.rs`, `transfer.rs` se dependente de run).
Manter em `arags-memory`: `HistoryManager` (repurpose), `ConsolidationEngine`, `decay`,
`PersistEngine`/`persist/`, `knowledge/`, `project.rs`, `watch.rs`, e o re-export
`pub use arags_core::memory::MemoryProvider`. A `MemoryProvider` trait em si é independente do
engine e deve permanecer.

### A.2 `arags-server`
- Remover `src/grpc/runs/` + `pub mod runs;` + RPCs `Run`/`StreamRun`/`GetRun`/`CancelRun`/
  `StreamRunEvents`/`GetRunStatus` (em `grpc/mod.rs`).
- Remover `src/grpc/summarize.rs` + `pub mod summarize;` + RPCs `Summarize*`/`StreamSummarize*`.
- Remover `src/summarizer/` (módulo inteiro).
- Remover `[llm]` (`config.rs` `LlmConfig`), `build_llm` (`lifecycle.rs`), campo `AppState.llm`,
  e a dependência `arags-llm` do `arags-server` (salvo se reutilizada em outro ponto — não há).
- Manter `GetServerStatus` (health/ops) mas remover as RPCs `Status`/`Cost`/`Cancel`/`Checkpoints`
  atreladas a run; elas somem junto com `runs`.

### A.3 `arags-proto`
- `proto/service.proto`: remover `rpc Run`, `rpc StreamRun`, `rpc GetRun`, `rpc CancelRun`,
  `rpc StreamRunEvents`, `rpc GetRunStatus`, `rpc Summarize*`, `rpc StreamSummarize*`.
- Remover mensagens órfãs: `RunRequest`, `RunOptions`, `RunResponse`, `RunStatus`,
  `RunEvent` (usada só por stream de run — remover; `Status`/`Cost` clientes não a usam mais),
  `SummarizeRequest/Response/Status/Progress`, `StreamSummarizeProgressResponse`.
- Manter `RunEvent`? Não — só servia ao stream de run. Remover.
- Regenerar via `build.rs`.

### A.4 `arags-cli`
- Remover `src/commands/run/` (dir inteiro) e a variante `Commands::Run` (`cli/commands.rs`).
- **`dispatch/local.rs` é removido por inteiro** (modo offline eliminado no plan 020 D3): o
  client vira **puro gRPC**. Sobra só `dispatch/server.rs`, cujo braço `Run` (e `Status { run_id }`
  / `Cost { run_id }` que chamava `get_run`) é removido.
- `commands/serve/`: remover `run_logic.rs`, `run_handler`/`run` rota em `handlers.rs`,
  `RunRequest` em `requests.rs`, e o `event_bus`/SSE de run em `events_stream` (fica só para
  status server, se aplicável — ou remove-se o SSE de run).
- Remover comandos: `session.rs`, `status.rs`, `cost.rs`, `cancel.rs`, `checkpoints.rs`,
  `restore_page.rs`, `wiki.rs`, `context.rs`, `consolidate.rs` (CLI), `decay.rs` (CLI),
  `entities.rs` (CLI), e os módulos `output/live_tree/`.
- `commands/mod.rs`: remover registros dos comandos acima.

### A.5 `arags-storage` (tabelas mortas)
As tabelas `runs`, `run_model_usage`, `trajectories`, `sessions` (multi-turn, migration 006 —
**distinta** de `auth_sessions` do plan 018), `session_contexts`, `session_history`,
`checkpoints` deixam de ser escritas. **Migrations são imutáveis** → não apagar; parar de usar e
agendar migration de drop em follow-up (documentado em Risks). O `qa_cache` (plan 017),
`result_cache` (purge admin), `auth_tokens`/`auth_sessions` (018), `chunks`/`buffers`/`files`
e `history` (repurpose) permanecem.

---

## B. Novo `arags init`

- Cria `<project>/.arags.toml` (idempotente; se existir, funde/avisa). Ver nomenclatura em
  plan 020 (`~/.arags/arags.toml` global, `.arags.toml` local, `server.toml` do server):
  ```toml
  [project]
  name = "<dir ou git remote>"
  ignore = ["target/", "node_modules/", ".git/", ...]   # semeado de .gitignore
  [server]
  addr = "http://127.0.0.1:50051"                        # default; sobrescreve global
  ```
- Por padrão executa `arags index .` ao final (flag `--no-index` p/ pular).
- Lê o global `~/.arags/arags.toml` (auth + LLM do plan 018/020) para validar identidade, mas não o
  copia (credencial fica no client).
- Semente ignore patterns de `.gitignore` presente; respeita o mesmo `ignore_patterns`/
  `force_include` do `index`.

---

## C. `arags memory` (admin) — unifica cache + decay + entities

Subcomando **admin-gated** (role `admin` do plan 018). Cliente manda `Authorization: Bearer`
(session); o interceptor exige `role=admin` (reuso do `require_admin`).

| Subcomando | RPC (server) | Ação |
|---|---|---|
| `list [--project P] [--limit N] [--include-entities]` | `ListMemory` | visualiza entradas `qa_cache` + stats + (opcional) entidades dos chunks |
| `get <cache_id>` | `GetCache` | busca resposta por id (admin/debug; usuário comum usa `arags query --cache-id`) |
| `invalidate [--cache-id C] [--project P] [--radius R] [--delete] [--reason S]` | `InvalidateCache` (plan 017/018) | marca `stale` (soft) ou `--delete` (hard); `--radius` invalida vizinhos; sem `--cache-id` purga `result_cache` legado do `--project` |
| `cleanup [--dry-run] [--project P]` | `TriggerMaintenance` | limpeza forçada de memória: **decay** (chunks com score < 0.1) + **consolidate** (dedupe + low-confidence) no server |

O `arags query --cache-id <id>` (consumer/anti-drift do plan 017) **permanece** sob `query`, não
sob `memory` — é o caminho determinístico do usuário final.

### C.1 Manutenção server-side (consolidate + decay)
- Novo módulo `arags-server/src/maintenance.rs`: `consolidate(project)` usa
  `arags_memory::ConsolidationEngine`; `decay(project)` usa `arags_search::decay::DecayConfig`
  (já existe) sobre `arags_storage`. Ambos retornam um `MaintenanceReport`
  (`duplicate_chunks_removed`, `low_confidence_patterns_removed`, `decayed_chunks`, `kept`).
- **Admin RPC** `TriggerMaintenance { project, dry_run }` → `MaintenanceReport`.
- **CLI interno** `arags-server admin consolidate [--dry-run] [--project P]` (abre `Storage`
  direto, sem gRPC — igual ao `admin create-refresh`).
- **Cron**: `arags-server` config ganha `[maintenance] interval_secs` (default ex.: 3600; `0` =
  desliga). Na `lifecycle`, um `tokio::spawn` dispara `consolidate+decay` a cada tick (e também
  logo após `index`, para manter a memória fresca).

---

## D. `arags persist` (rework)

- Uso: `arags persist <response_id> [--title T]`. `<response_id>` é o `cache_id` emitido por
  `arags query -qa` (plan 017 já emite `cache_id` em todos os formatos).
- Fluxo:
  1. Exibe o `response_id` ao usuário.
  2. `GetAnswerById(response_id)` → `answer_text` + `source_chunk_ids` + arquivos (provenance).
  3. Executa resumo **via IA real do usuário** (`arags-llm` + `~/.arags/arags.toml`, igual ao `query -qa`)
     com prompt de sumarização fixo + os artefatos (trechos fonte), produzindo um documento.
  4. Escreve `wiki/<yyyymmddhhmm>_<slug(title)>.md` (pasta `wiki/` relativa ao projeto; sem git).
- **Estrutura fixa** do documento:
  ```markdown
  # <title>

  - **Response ID:** <cache_id>
  - **Generated:** <yyyymmddhhmm>
  - **Model:** <model>
  - **Project:** <project>
  - **Provenance:** <chunk_ids / file_paths>

  ## Summary
  <resumo gerado pela IA>

  ## Key Findings / Artifacts
  <lista estruturada de arquivos fonte + snippets relevantes>

  ## Related
  <refs / queries derivadas>
  ```
- Remove-se `save_page` (usado por run/context) e o uso de `PersistEngine` solto; `persist`
  vira o único dono da pasta `wiki/`.

---

## E. `arags history` (por usuário, atrelado ao refresh token)

- Atual `HistoryManager` (cliente, lê SQLite local) → **server-side**: o server registra cada
  query em `history` com a coluna `user` (do `AuthContext` do plan 018). Migration adiciona
  `user TEXT` a `history`.
- RPC `GetHistory { user, limit }`: usuário vê **só suas** queries; `admin` vê todas (ou filtra
  por `user`). Sem sessão → `UNAUTHENTICATED`.
- `arags history [--limit N]` (default 20) chama `GetHistory` com o `username` do token; remove a
  seção "runs" (morta).
- Respeita o princípio "histórico útil a nível do próprio usuário, atrelado ao refresh token".

---

## F. Justificativas das remoções

- **`session`**: as `sessions` (multi-turn, migration 006) existiam para rastrear contextos/
  turnos de um **run RLM de longa duração**. Com on-demand `-qa` não há sessão multi-turno a
  rastrear; o `history` por usuário já cobre o que o usuário precisa. Removem-se comando, RPCs e
  tabelas `sessions`/`session_contexts`/`session_history`.
- **`context`**: `arags context <task>` monta janela de contexto (busca híbrida) sem LLM — é
  **redundante** com `arags query` quando nenhum `--llm` está configurado (query já devolve o
  contexto). Consolidar em `query`.
- **`status`/`cost`/`cancel`/`checkpoints`**: 100% atrelados a `runs`/`trajectories` (tabelas
  mortas). Sem o engine, não têm objeto.
- **`restore-page`**: reconstruía página a partir de busca FTS — desnecessário com o `persist`
  estruturado e o `query`.
- **`wiki` (comando)**: gestão de repo git de knowledge base **não é responsabilidade do
  projeto**. O `persist` apenas escreve arquivos markdown numa pasta `wiki/` (sem git).
- **`consolidate`/`decay` (CLI)**: lógica determinística que pertence ao **servidor** (dono do
  DB compartilhado) e deve rodar periodicamente (cron), não sob demanda no client de cada dev.

---

## Data Model (mudanças)

| Tabela | Mudança |
|---|---|
| `history` | adicionar coluna `user TEXT` (migration); server escreve com `username` do token |
| `qa_cache` | inalterada (plan 017) |
| `result_cache` | inalterada (purge admin via `memory invalidate` sem `--cache-id`) |
| `runs`,`run_model_usage`,`trajectories`,`sessions`,`session_*`,`checkpoints` | **mortas**: parar de escrever; drop agendado em follow-up |
| `auth_tokens`,`auth_sessions` | inalteradas (plan 018) |

Novas RPCs: `ListMemory`, `GetCache`, `TriggerMaintenance`, `GetHistory`. Removidas:
`Run*`, `Summarize*`, `GetRun`, `CancelRun`, `StreamRunEvents`, `GetRunStatus`, `Status[run]`,
`Cost[run]` (as do run).

---

## Where to Implement

| Componente | Crate | Arquivo(s) |
|---|---|---|
| Remoção engine | `arags-core` | `src/{engine,planner,solver,synthesizer,repl,budget,cache,tools,router,sampling,guardrails,compaction,docker,jsonl_logger,token_counter,concurrency,events}.rs`, `src/lib.rs`, `src/types/{mod,enums,input,node}.rs` |
| Podar `arags-memory` run-bound | `arags-memory` | remover `engine/memory_api.rs`, `trajectory/`, `checkpoint/`, `session.rs`, `transfer.rs` (se run-bound); manter `HistoryManager`, `ConsolidationEngine`, `decay`, `PersistEngine`, `knowledge`, `project`, `watch` |
| Remover runs/summarize/`[llm]` | `arags-server` | `grpc/{mod,runs,summarize}.rs`, `summarizer/`, `config.rs`, `lifecycle.rs`, `state.rs` |
| Manutenção server-side | `arags-server` | `src/maintenance.rs` (novo) + `admin consolidate` em `cli/admin.rs` + ticker em `lifecycle.rs` |
| RPCs memory/history/maintenance | `arags-proto`+`arags-server` | `proto/service.proto`, `grpc/{query_cache,memory,history}.rs` (novos/ajustados) |
| `arags init` | `arags-cli` | `src/commands/init.rs` (novo) + `cli/commands.rs` |
| `arags memory` | `arags-cli` | `src/commands/memory.rs` (novo, subs. cache/decay/entities) |
| `arags persist` rework | `arags-cli` | `src/commands/persist.rs` (rewrite) |
| `arags history` por usuário | `arags-cli`+`arags-server` | `src/commands/history.rs` (rewrite) + `grpc/history.rs` + `HistoryManager` server-side |
| Remoções CLI | `arags-cli` | deletar `run/`, `session`, `status`, `cost`, `cancel`, `checkpoints`, `restore_page`, `wiki`, `context`, `consolidate`, `decay`, `entities`; `dispatch/{local,server}.rs`; `commands/serve/{run_logic,handlers,requests}`, `output/live_tree` |
| Migrations | `arags-storage` | migration `history.user`; parar writes das tabelas mortas |

---

## Implementation Steps (milestones)

1. **Core**: remover módulos engine + podar `types`/`lib.rs`; ajustar `arags-memory` (remover
   run-bound, manter trait `MemoryProvider`).
2. **Proto**: remover RPCs `Run*`/`Summarize*` + mensagens órfãs; adicionar `ListMemory`,
   `GetCache`, `TriggerMaintenance`, `GetHistory`; regenerar.
3. **Server**: remover `grpc/runs`, `grpc/summarize`, `summarizer`, `[llm]`/`build_llm`;
   implementar `maintenance.rs` (consolidate+decay) + ticker + `admin consolidate`; implementar
   handlers `memory`/`history`; manter `GetServerStatus` (health).
4. **CLI removals**: deletar comandos run-bound + `session/status/cost/cancel/checkpoints/
   restore-page/wiki/context/consolidate/decay/entities`; limpar `dispatch` e `serve`.
5. **CLI novos/rework**: `init`, `memory` (admin), `persist` (response_id→wiki), `history`
   (por usuário); refinar `query` (já emite `cache_id`; `--cache-id` lookup).
6. **Storage**: migration `history.user`; parar escrita das tabelas mortas.
7. **Wire auth**: `memory`/`history` admin/user gates reusam `authenticate`+`require_admin`
   (plan 018).
8. **`cargo check --workspace` + `clippy -D warnings` + `fmt`** iterativos até verde.
9. **Docs**: atualizar READMEs/MODULE/CHANGELOG/ARCHITECTURE para a nova filosofia on-demand e
   nova superfície (`init/index/search/query/memory/persist/history`).

---

## Testing & Benchmarks

- `test_no_run_command_compiles` (variant `Run` ausente em `cli/commands.rs`).
- `test_server_has_no_llm_config` (`[llm]` inexistente em `config.rs`; `AppState.llm` ausente).
- `test_memory_invalidate_admin_only` (non-admin → `PERMISSION_DENIED`; admin passa) — estende
  018.
- `test_memory_cleanup_dry_run_noop` (report correto, sem delete).
- `test_memory_cleanup_decay_removes_stale` + `test_memory_cleanup_consolidates_dupes`.
- `test_history_scoped_per_user` (user A não vê queries de B; admin vê todas).
- `test_history_requires_auth` (sem sessão → `UNAUTHENTICATED`).
- `test_persist_writes_wiki_md` (estrutura fixa presente; `response_id` exibido; usa LLM do
  usuário).
- `test_init_idempotent` (segunda chamada não sobrescreve; roda `index`).
- `test_init_seeds_ignore_from_gitignore`.
- `test_query_emits_cache_id` (plan 017) + `test_query_cache_id_lookup_1to1` (já existe).
- `test_proto_no_run_or_summarize_messages`.
- `test_arags_memory_replaces_cache_decay_entities` (CLI mapping).
- Bench: latência de `TriggerMaintenance` em projeto grande; tamanho do binário (sem engine).

---

## Risks

| Risco | Mitigação |
|---|---|
| `arags-memory` acoplado a tipos do engine (`RlmNode`/`RlmRunResult`/`StartRunInput`) | remover `memory_api.rs` + módulos run-bound; a trait `MemoryProvider` é independente e fica |
| Tabelas mortas (`runs`, etc.) não dropadas por migrations imutáveis | parar de escrever agora; migration de drop em follow-up documentada; nenhum código as referencia |
| `history` local → server-side quebra clientes antigos | migration adiciona `user` (nullable p/ retroativo); server preenche a partir do token |
| `memory cleanup` (decay) agressivo apaga chunks úteis | `DecayConfig` score < 0.1 default + `--dry-run` reporta antes de apagar |
| Consolidate periódico compete com `index` | lifecycle serializa (um por vez) ou usa o mesmo lock de buffer |
| `persist` depende de LLM do usuário configurado | igual ao `query -qa`; sem config, erro claro "configure [llm] em ~/.arags/arags.toml" |
| Perda de `session` põe em risco continuity de agentes | `history` por usuário cobre rastreabilidade; não há multi-turno no modelo on-demand |
| Binário/cliente quebra por reexport de `arags_core::types` | `cargo check --workspace` após poda de `types`; manter só o que sobrevive |

---

## Relação com 017/018

- **Sobre 017 (QA-Cache):** não altera o cache em si; apenas **reempacota a exposição** —
  `cache` vira `memory` (admin) e o `query --cache-id` continua sendo o lookup determinístico do
  usuário. `InvalidateCache` (admin-gated) é preservado e ganha `cleanup`.
- **Sobre 018 (auth):** `memory` e `history` reutilizam `authenticate`/`require_admin` e o
  `username` do refresh token para escopo e auditoria. Nenhuma mudança em `tokens`/`sessions`
  (auth).
- **Remove o que 017/018 tornaram obsoleto:** o `summarizer` (server-LLM, oposto ao 017) e o
  `run` (loop recursivo, oposto ao on-demand).

## Relação com 020 (Config Consolidation)

- O **plan 020** é o complemento de config desta refatoração. Ele define:
  - `server.toml` (**só** data plane: listen/tls, data_dir/pool, `[embedder]` chunk/embed,
    `[search]`, `[qa_cache]`, `[maintenance]`) — sem `[llm]`.
  - User 2-escopos: global `~/.arags/arags.toml` (auth + llm user + server.addr) e local
    `.arags.toml` (overrides por projeto, gitignored, gerado por `arags init`), com **merge
    granular** local→global.
  - As referências a `~/.arags/config.toml`/`.arags/config.toml` neste plano 019 estão **supersedidas**
    pela nomenclatura do 020.
- `arags init` (seção B) e `arags memory`/`persist` (C/D) consomem `user_config` do 020.
- `[maintenance]` (C.1) é configurado no `server.toml` do 020.
