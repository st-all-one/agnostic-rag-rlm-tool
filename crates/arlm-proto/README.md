# arlm-proto

Definições Protobuf e tipos gerados para a comunicação gRPC do arlm (cliente ↔ servidor).

## Responsabilidades

- **Contrato gRPC**: fonte única da verdade para a comunicação cliente/servidor via tonic.
- **Schema versionado**: pacote `arlm.v1` (versionamento explícito de proto).
- **Codegen**: `build.rs` compila os `.proto` via `tonic_build` em tipos Rust (prost) em tempo de build.
- **Validação**: testes de integração em `tests/` verificam mensagens, enums e acessores gerados.

## Estrutura

```
proto/
├── project.proto      # CreateProjectRequest, ProjectInfo, ListProjectsResponse
├── index.proto        # IndexRequest, IndexResponse
├── search.proto       # SearchRequest, SearchTier, SearchResult, SummaryInfo, SearchResponse
├── context.proto      # ContextRequest, ContextResponse, ContextStats
├── run.proto          # RunRequest, RunOptions, RunResponse, RunStatus, RunResult, RunStats, RunEvent
├── session.proto      # CreateSessionRequest, SessionInfo, ListSessionsResponse, SessionTurn, AddSessionTurnRequest
├── summarize.proto    # SummaryScope, SummaryChunk, Summarize*, SummaryStatus, StaleSummary
├── server.proto       # ServerStatus, WriteQueueStats, SummarizeStatus
└── service.proto      # service ArlmService (18 RPCs)
build.rs               # tonic_build: compila os 9 sub-arquivos + timing/logs
src/lib.rs             # pub mod proto { include!(arlm.v1.rs) }; pub use proto::*;
tests/proto_contract.rs# 6 testes de integração validando o contrato gerado
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

- `RunResult.total_cost` (`run.proto`, campo 5): lido diretamente pelo CLI (`run.total_cost`).
- `SessionInfo` / `AddSessionTurnRequest`: campos atuais já batem com servidor/cliente (sem mismatch).
