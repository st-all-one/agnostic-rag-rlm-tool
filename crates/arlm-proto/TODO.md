# TODO — arlm-proto

> Definição protobuf e código gerado (prost + tonic).

## Status Atual

Proto dividido em sub-arquivos sob `proto/` (project, index, search, context,
session, server, auth, query_cache, service), compilado por `build.rs` via
`tonic_build`. `package arlm.v1;` para versionamento. Código gerado validado
por testes de integração em `tests/proto_contract.rs`.

> **OBSOLETO (pós plano 019):** `run.proto` e `summarize.proto` foram removidos;
> `context.proto`/`session.proto` estão em desuso. Veja `plan/019-cli-consolidation.md`.

---

## Gaps (resolvidos / concluídos)

### 1. Campos faltando no proto — PARCIALMENTE RESOLVIDO
- **`RunResult.total_cost`** — RESOLVIDO: adicionado `double total_cost = 5;`
  em `proto/run.proto`. O literal exaustivo em
  `crates/arlm-server/src/grpc/runs/mod.rs` agora define `total_cost`, e o CLI
  (`main.rs:513` `run.total_cost`) compila.
- **`SessionInfo.updated_at`** — CONCLUÍDO/IGNORADO: investigado; o servidor
  (`session.rs`) NÃO define `updated_at` em `SessionInfo` e não há quebra de
  compilação. O banco armazena a coluna, mas o proto não a expõe hoje. Sem
  mismatch real → não alterado.
- **`AddSessionTurnRequest`** — CONCLUÍDO/IGNORADO: campos do proto
  (`session_id`, `query`, `response`) já batem com o uso no servidor/cliente.
  Sem mismatch → não alterado.

### 2. Sem validação de proto — RESOLVIDO
- Adicionado `tests/proto_contract.rs` com 6 testes de integração que validam
  mensagens, enums e acessores (RunResult+total_cost, SearchRequest+TierHybrid,
  SessionInfo, AddSessionTurnRequest, variantes de enum, módulos de serviço).
  `cargo test -p arlm-proto` passa.

### 3. Sem versionamento de proto — RESOLVIDO
- `package arlm;` → `package arlm.v1;` em todos os sub-proto. Isso gera
  `arlm.v1.rs`; `lib.rs` faz `include!` desse arquivo dentro de `mod proto`,
  preservando `arlm_proto::proto::*`,
  `arlm_proto::proto::arlm_service_server::ArlmService` e
  `arlm_proto::proto::arlm_service_client::ArlmServiceClient`. (Nota: os módulos
  de serviço gerados permanecem `arlm_service_{client,server}`, então nenhum
  alias de re-export foi necessário — as importações downstream continuam
  válidas.)

---

## Pendências fora do escopo (não-proto)
- `arlm-core/src/engine/mod.rs:48` — `run_rlm_engine_with_events` passou a exigir
  4 argumentos (o 4º é `memory: Option<Arc<dyn MemoryProvider>>`, adicionado na
  refatoração do `arlm-core`). Os 3 chamadores
  (`arlm-cli` `commands/run.rs`, `commands/serve.rs`; `arlm-server`
  `grpc/runs/engine.rs`) foram corrigidos para passar `None`. Workspace compila
  (`cargo check -p arlm-cli -p arlm-server` verde). Resolvido fora deste crate.
