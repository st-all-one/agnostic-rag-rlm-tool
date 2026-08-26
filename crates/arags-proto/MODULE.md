# arags-proto

## O que faz
Crate de definições Protobuf + código gerado (prost/tonic) que define o contrato gRPC cliente↔servidor do arags. É a fonte única da verdade para a comunicação entre `arags-cli` (cliente gRPC puro) e `arags-server` (plano de dados LLM-free).

## Estrutura
- `proto/*.proto` — schema dividido em sub-arquivos coesos (cada um < 300 linhas): `project`, `index`, `search`, `context`, `session`, `server`, `auth`, `query_cache`, `exploration`, `rlm`, `service`. Todos com `package arags.v1;`. (`run.proto` e `summarize.proto` foram removidos — não há mais runs de RLM nem sumarização server-side.) **plan 023:** `search.proto` carrega os campos aditivos da unified query (`SearchResponse.summaries: SummaryHit`, `.explorations: ExplorationRef`) e `exploration.proto` o RPC de review (`ReviewExploration`).
- `build.rs` — `tonic_build::configure().build_server(true).build_client(true).compile_protos(&[...], &["proto"])`; compila os sub-arquivos e emite log estruturado de tempo de execução via `std::time::Instant` + `eprintln!`.
- `src/lib.rs` — `pub mod proto { include!(concat!(env!("OUT_DIR"), "/arags.v1.rs")); } pub use proto::*;`. O módulo `proto` carrega `#![allow(clippy::all, clippy::pedantic, clippy::cargo, clippy::nursery, dead_code, missing_docs)]` para isolar os lints do código gerado.
- `tests/proto_contract.rs` — 6 testes de integração validando mensagens, enums, acessores e os módulos de serviço (`arags_service_{client,server}`).

## Dependências
- Internas: nenhuma (crate folha de contrato; consumido por `arags-server` e `arags-cli`).
- Externas (runtime): `prost`, `prost-types`, `tonic`, `http`.
- Externas (build): `prost-build`, `tonic-build`.

## Convenções deste módulo
- O `.proto` é a fonte da verdade; os tipos gerados NÃO são editados à mão (ficam em `OUT_DIR`/`target`).
- `package arags.v1` garante versionamento explícito; mudanças breaking exigem novo pacote (ex.: `arags.v2`).
- Testes de integração em `tests/` validam o contrato gerado; usam `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` no topo.
- Nunca quebrar os caminhos downstream: `arags_proto::proto::*`,
  `arags_proto::proto::arags_service_server::AragsService` e
  `arags_proto::proto::arags_service_client::AragsServiceClient`. Se o nome do
  módulo gerado mudar (ex.: ao trocar o `package`), usar re-export em `lib.rs`.
- Sem `unwrap`/`expect`/`panic` em `src/`/`build.rs`; `build.rs` usa `?` sobre `std::io::Result`.

## Comandos úteis
```bash
CARGO_BUILD_JOBS=4 cargo check  -p arags-proto
CARGO_BUILD_JOBS=4 cargo test   -p arags-proto   # 6 testes de contrato
CARGO_BUILD_JOBS=4 cargo clippy -p arags-proto --all-targets -- -D warnings
```

## Migrations
- N/A — o proto não possui schema de banco; versionamento é feito via `package arags.v1` (e evolução para `arags.v2` em breaking changes).

## Rules
- Ao adicionar campo a uma mensagem já construída por literal exaustivo em `arags-server`/`arags-cli`, atualizar também o literal correspondente (ou documentar por que não).
- Mantenha `arags_proto::proto::*` e `arags_service_{client,server}` válidos; use re-export em `lib.rs` se o módulo gerado mudar de nome.
- `build.rs` deve sempre logar `stage=compile_protos duration_ms=... files=...`.
- Valide o contrato com `cargo test -p arags-proto` após qualquer mudança no `.proto`.
