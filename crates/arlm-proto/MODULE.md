# arlm-proto

## O que faz
Crate de definições Protobuf + código gerado (prost/tonic) que define o contrato gRPC cliente↔servidor do arlm. É a fonte única da verdade para a comunicação entre `arlm-cli`, `arlm-server` e os eventos de run/summarize.

## Estrutura
- `proto/*.proto` — schema dividido em 9 sub-arquivos coesos (cada um < 300 linhas): `project`, `index`, `search`, `context`, `run`, `session`, `summarize`, `server`, `service`. Todos com `package arlm.v1;`.
- `build.rs` — `tonic_build::configure().build_server(true).build_client(true).compile_protos(&[...], &["proto"])`; compila os 9 sub-arquivos e emite log estruturado de tempo de execução via `std::time::Instant` + `eprintln!`.
- `src/lib.rs` — `pub mod proto { include!(concat!(env!("OUT_DIR"), "/arlm.v1.rs")); } pub use proto::*;`. O módulo `proto` carrega `#![allow(clippy::all, clippy::pedantic, clippy::cargo, clippy::nursery, dead_code, missing_docs)]` para isolar os lints do código gerado.
- `tests/proto_contract.rs` — 6 testes de integração validando mensagens, enums, acessores e os módulos de serviço (`arlm_service_{client,server}`).

## Dependências
- Internas: nenhuma (crate folha de contrato; consumido por `arlm-server` e `arlm-cli`).
- Externas (runtime): `prost`, `prost-types`, `tonic`, `http`.
- Externas (build): `prost-build`, `tonic-build`.

## Convenções deste módulo
- O `.proto` é a fonte da verdade; os tipos gerados NÃO são editados à mão (ficam em `OUT_DIR`/`target`).
- `package arlm.v1` garante versionamento explícito; mudanças breaking exigem novo pacote (ex.: `arlm.v2`).
- Testes de integração em `tests/` validam o contrato gerado; usam `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` no topo.
- Nunca quebrar os caminhos downstream: `arlm_proto::proto::*`,
  `arlm_proto::proto::arlm_service_server::ArlmService` e
  `arlm_proto::proto::arlm_service_client::ArlmServiceClient`. Se o nome do
  módulo gerado mudar (ex.: ao trocar o `package`), usar re-export em `lib.rs`.
- Sem `unwrap`/`expect`/`panic` em `src/`/`build.rs`; `build.rs` usa `?` sobre `std::io::Result`.

## Comandos úteis
```bash
CARGO_BUILD_JOBS=4 cargo check  -p arlm-proto
CARGO_BUILD_JOBS=4 cargo test   -p arlm-proto   # 6 testes de contrato
CARGO_BUILD_JOBS=4 cargo clippy -p arlm-proto --all-targets -- -D warnings
```

## Migrations
- N/A — o proto não possui schema de banco; versionamento é feito via `package arlm.v1` (e evolução para `arlm.v2` em breaking changes).

## Rules
- Ao adicionar campo a uma mensagem já construída por literal exaustivo em `arlm-server`/`arlm-cli`, atualizar também o literal correspondente (ou documentar por que não).
- Mantenha `arlm_proto::proto::*` e `arlm_service_{client,server}` válidos; use re-export em `lib.rs` se o módulo gerado mudar de nome.
- `build.rs` deve sempre logar `stage=compile_protos duration_ms=... files=...`.
- Valide o contrato com `cargo test -p arlm-proto` após qualquer mudança no `.proto`.
