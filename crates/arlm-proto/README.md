# arlm-proto

Definições Protobuf e tipos gerados para a comunicação gRPC do arlm (cliente ↔ servidor).

## Responsabilidades

- **Contrato gRPC**: fonte única da verdade para a comunicação cliente/servidor via tonic.
- **Schema versionado**: pacote `arlm.v1` (versionamento explícito de proto).
- **Auth (plan 018)**: `auth.proto` + `AuthRefresh` RPC — refresh tokens com rotação e
  sessões de curta duração (roles `Admin`/`NonAdmin`).
- **Query-Answer Cache (plan 017)**: `query_cache.proto` + `QueryWithCache`,
  `StoreAnswer`, `GetAnswerById`, `InvalidateCache` (single + cluster por raio).
- **Codegen**: `build.rs` compila os `.proto` via `tonic_build` em tipos Rust (prost) em tempo de build.
- **Validação**: testes de integração em `tests/` verificam mensagens, enums e acessores gerados.

## Estrutura

```
proto/
├── project.proto      # CreateProjectRequest, ProjectInfo, ListProjectsResponse
├── index.proto        # IndexRequest, IndexResponse (client-streaming de texto)
├── search.proto       # SearchRequest, SearchTier, SearchResult, SummaryInfo, SearchResponse
├── context.proto      # ContextRequest, ContextResponse, ContextStats (legacy)
├── session.proto      # CreateSessionRequest, SessionInfo, ListSessionsResponse (legacy)
├── server.proto       # ServerStatus, ServerStatusRequest
├── auth.proto         # AuthRefreshRequest/Response (plan 018)
├── query_cache.proto  # QueryWithCache/StoreAnswer/GetAnswerById/InvalidateCache (plan 017)
└── service.proto      # service ArlmService (RPCs atuais, inclui ListMemory/GetCache/
                       #   TriggerMaintenance/GetHistory definidos inline)
build.rs               # tonic_build: compila os sub-arquivos + timing/logs
src/lib.rs             # pub mod proto { include!(arlm.v1.rs) }; pub use proto::*;
tests/proto_contract.rs# testes de integração validando o contrato gerado
```

## Versionamento

O schema usa `package arlm.v1;`. O `lib.rs` faz `include!` do arquivo gerado
`arlm.v1.rs` dentro de `mod proto`, preservando os caminhos downstream:
`arlm_proto::proto::*`, `arlm_proto::proto::arlm_service_server::ArlmService` e
`arlm_proto::proto::arlm_service_client::ArlmServiceClient`.

## Codegen / Observabilidade

`build.rs` emite log estruturado com tempo de execução:

```
[arlm-proto/build] stage=compile_protos duration_ms=<ms> files=<n>
```

## Uso (downstream)

```rust
use arlm_proto::proto::*;
use arlm_proto::proto::arlm_service_client::ArlmServiceClient;
use arlm_proto::proto::arlm_service_server::ArlmService;
```

## Testes

```bash
CARGO_BUILD_JOBS=4 cargo test -p arlm-proto   # 6 testes de contrato
```

## Campos do contrato (notas)

- Os RPCs de memória/histórico/manutenção (`ListMemory`, `GetCache`,
  `TriggerMaintenance`, `GetHistory`) são definidos em `service.proto`.
- `run.proto` e `summarize.proto` **foram removidos** (o servidor é LLM-free e não
  há mais runs de RLM nem sumarização server-side). `context.proto`/`session.proto`
  permanecem mas estão em desuso (os comandos CLI `context`/`session` foram removidos
  no plan 019).
