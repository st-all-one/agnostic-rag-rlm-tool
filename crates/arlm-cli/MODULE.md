# arlm-cli

## O que faz
CLI *agent-agnostic* do `arlm`: faz o parsing de argumentos (clap), resolve a
configuração do usuário (**2 escopos**: `~/.arlm/arlm.toml` global +
`.arlm.toml` local, com merge granular por campo) e roteia cada subcomando para
um `arlm-server` remoto via gRPC. É um **cliente gRPC puro**: não há modo local
(plan 020 removeu o subcomando `serve`/MCP e o data plane local). Usa o **LLM local do usuário** (`arlm-llm`) apenas para *digest*
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
- `src/auth_client.rs` — `ArlmClient` autenticado (`AuthRefresh` + interceptor
  Bearer com renovação em background).
- `src/backend.rs` — resolve o backend LLM do usuário a partir de
  `[[llm.backends]]` (usado por `query -qa` e `persist`).
- `src/client.rs` — `ClientConfig` + `connect_channel` (retry/backoff, validação
  de endereço, TLS automático em `https://` e mTLS via `[server].tls_ca`/
  `tls_cert`/`tls_key`).
- `src/user_config.rs` — config 2-escopos (`[auth]` só-global, `[llm]`,
  `[server]` com knobs TLS, `[project]`); merge granular testado inline;
  arquivos legados `config.toml` **não** são lidos.
- `src/commands/` — módulos de comando:
  - `qa_cache` (plan 017: `run_ask`/`run_get`/`run_invalidate` orquestrando os
    RPCs `QueryWithCache`/`GetAnswerById`/`InvalidateCache`; a digestão LLM roda
    localmente via `arlm-llm`/`user_config` e o `StoreAnswer` é fire-and-forget),
  - `persist` (escreve `wiki/*.md` via LLM do usuário).
  - `index`, `search`, `query`, `memory` (admin), `history` vivem em
    `dispatch/server.rs` (streaming de arquivos + renderização).
- `src/cli/commands.rs` — `Commands` enum (inclui `Query` estendido com
  `cache_id`/`qa` e o subcomando `Memory`).
- `src/output/` — `mod` (`Format`), `json`, `tree`, `markdown`, `prompt`.
- `tests/` — testes de integração (um arquivo por módulo); sem `#[cfg(test)]`
  em `src/`.

## Dependências
- Internas: `arlm-core`, `arlm-llm`, `arlm-proto` (plan 020: sem
  `arlm-storage`/`arlm-search`/`arlm-memory` — o client nunca abre estado local;
  guardado por teste em `tests/init_test.rs`).
- Externas: `clap` (derive), `tokio`/`tokio-stream` (async/streaming),
  `tonic` (gRPC), `tracing`/`tracing-subscriber` (logs), `serde`/`serde_json`/
  `toml` (config/saída), `anyhow` (erros), `indicatif`/`console` (UI),
  `chrono` (timestamps do wiki), `parking_lot` (sync), `mimalloc` (allocator).

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
- Testes de API pública ficam em `tests/`; funções puras críticas (merge da
  config) têm `#[cfg(test)]` inline com tempdirs.

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
- Padrão de produção: `dispatch::dispatch(cli, &rt)` carrega a user_config e
  roteia tudo para o servidor gRPC; nenhum comando abre Storage local.
- O CLI é um **cliente gRPC puro**: todos os comandos (`init`, `index`,
  `search`, `query`, `memory`, `persist`, `history`, `server`) vão para o
  `arlm-server` (plano de dados, LLM-free).
- `--llm` NÃO existe no CLI: o LLM do usuário é usado implicitamente em
  `query -qa` (digest) e `persist` (summarize) via `arlm-llm`.
- A config do usuário é 2-escopos (`user_config.rs`): `~/.arlm/arlm.toml`
  (global, com `[auth]` só-global) + `.arlm.toml` (local, merge por campo,
  `[project]`). `server.addr` alimenta o cliente.
