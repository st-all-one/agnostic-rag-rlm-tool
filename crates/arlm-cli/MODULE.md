# arlm-cli

## O que faz
CLI *agent-agnostic* do `arlm`: faz o parsing de argumentos (clap), resolve a
configuração do usuário (**2 escopos**: `~/.arlm/arlm.toml` global +
`.arlm.toml` local, com merge granular por campo) e roteia cada subcomando para
um `arlm-server` remoto via gRPC (`--server`). É um **cliente gRPC puro**: não
há modo local. Usa o **LLM local do usuário** (`arlm-llm`) apenas para *digest*
(`query -qa`) e *summarize* (`persist`). O servidor é um plano de dados puro
(LLM-free). Renderiza saídas em 4 formatos (`json`, `tree`, `markdown`, `prompt`)
com logs estruturados (`tracing`).

## Estrutura
- `src/lib.rs` — API pública (re-exports) + allows de lint (pedantic estilo).
- `src/main.rs` — *thin binary*: parse → `init_logging` → `dispatch`.
- `src/cli/` — `Cli`, `Commands`, `OutputFormatArg` (clap derive, desacoplado
  do entry point).
- `src/dispatch/` — `mod` (resolução de config + branch para o servidor),
  `server` (modo gRPC, renderiza respostas conforme `--format`). Não há modo
  local — todo comando vai para o servidor.
- `src/client.rs` — `ClientConfig` + `create_client` (retry/backoff, validação
  de endereço, TLS automático).
- `src/user_config.rs` — config 2-escopos (`[auth]` só-global, `[llm]`,
  `[server]`, `[project]`); arquivos legados `config.toml` **não** são lidos.
- `src/util.rs` — `data_dir()`, resolução de projeto.
- `src/commands/` — um módulo por subcomando:
  - `serve/` — `arlm server` (gRPC/MCP data plane).
  - `index`, `search`, `query`, `qa_cache` (plan 017: `run_ask`/`run_get`/
    `run_invalidate` orquestrando os RPCs `QueryWithCache`/`GetAnswerById`/
    `InvalidateCache`; a digestão LLM roda localmente via `arlm-llm`/`user_config`
    e o `StoreAnswer` é fire-and-forget), `memory` (admin: list/get/invalidate/
    cleanup → ListMemory/GetCache/InvalidateCache/TriggerMaintenance),
    `persist` (escreve `wiki/*.md` via LLM do usuário), `history`.
- `src/cli/commands.rs` — `Commands` enum (inclui `Query` estendido com
  `cache_id`/`qa` e o subcomando `Memory`).
- `src/output/` — `mod` (`Format`), `json`, `tree`, `markdown`, `prompt`.
- `tests/` — testes de integração (um arquivo por módulo); sem `#[cfg(test)]`
  em `src/`.

## Dependências
- Internas: `arlm-core`, `arlm-storage`, `arlm-search`, `arlm-memory`,
  `arlm-llm`, `arlm-embedding`, `arlm-proto`.
- Externas: `clap` (derive), `tokio` (async), `tonic`/`prost` (gRPC),
  `axum`/`tower-http` (HTTP/MCP), `tracing`/`tracing-subscriber` (logs),
  `serde`/`tomoml` (config), `anyhow` (erros), `indicatif`/`console` (UI),
  `mimalloc` (allocator), `parking_lot` (sync), `uuid`/`chrono`.

## Convenções deste módulo
- Sem `unwrap`/`expect`/`panic`/`unsafe` em `src/`; use `anyhow` + `?`.
- Logs estruturados via `tracing` (`info!`/`debug!`/`warn!` com campos) e
  *timing* via `std::time::Instant` registrado como `elapsed_ms`.
- Segurança de thread: estado compartilhado é `Send + Sync`
  (`Arc` + `parking_lot::Mutex`/`RwLock`).
- Performance: evitar clones desnecessários; `with_capacity` quando o tamanho
  é conhecido.
- `dispatch` é o único ponto que conhece a árvore de comandos; `commands::*`
  expõe `execute(Config)` estável.
- Testes de API pública ficam em `tests/`; `src/` não contém `#[cfg(test)]`.

## Comandos úteis
```bash
CARGO_BUILD_JOBS=4 cargo check   -p arlm-cli --all-targets
CARGO_BUILD_JOBS=4 cargo clippy   -p arlm-cli --all-targets -- -D warnings
CARGO_BUILD_JOBS=4 cargo test     -p arlm-cli
cargo fmt -p arlm-cli -- --check
```

## Migrations
- N/A — o crate não possui schema próprio (estado em `arlm-storage`/`arlm-memory`).

## Rules
- Padrão de produção: `dispatch::dispatch(cli, cfg)` resolve tudo e roteia para
  o servidor gRPC.
- O CLI é um **cliente gRPC puro**: todos os comandos (`init`, `index`,
  `search`, `query`, `memory`, `persist`, `history`, `server`) vão para o
  `arlm-server` (plano de dados, LLM-free).
- `--llm` NÃO existe no CLI: o LLM do usuário é usado implicitamente em
  `query -qa` (digest) e `persist` (summarize) via `arlm-llm`.
- A config do usuário é 2-escopos (`user_config.rs`): `~/.arlm/arlm.toml`
  (global, com `[auth]` só-global) + `.arlm.toml` (local, merge por campo,
  `[project]`). `server.addr` alimenta o cliente.
