# arlm-cli

## O que faz
CLI *agent-agnostic* do `arlm`: faz o parsing de argumentos (clap), resolve a
configuração (CLI > `config.toml` > defaults) e roteia cada subcomando para a
implementação local ou para um `arlm-server` remoto via gRPC (`--server`).
Cobra busca híbrida, RLM recursivo, gestão de sessões/memória e um servidor
HTTP/MCP. Renderiza saídas em 4 formatos (`json`, `tree`, `markdown`, `prompt`)
com logs estruturados (`tracing`) e *timing* de fases.

## Estrutura
- `src/lib.rs` — API pública (re-exports) + allows de lint (pedantic estilo).
- `src/main.rs` — *thin binary*: parse → `init_logging` → `dispatch`.
- `src/cli/` — `Cli`, `Commands`, `SessionAction`, `OutputFormatArg`,
  `parse_tool_arg` (clap derive, desacoplado do entry point).
- `src/dispatch/` — `mod` (resolução de config + branch local/servidor),
  `local` (execução local chamando `commands::*`), `server` (modo gRPC,
  renderiza respostas conforme `--format`).
- `src/client.rs` — `ClientConfig` + `create_client` (retry/backoff, validação
  de endereço, TLS automático).
- `src/config.rs` — `Config` (TOML) com seções `embedding`, `search`, `agent`,
  `server`.
- `src/metrics.rs` — `ArlmMetrics` (formato Prometheus, `Send + Sync`).
- `src/util.rs` — `data_dir()`, `project_name()`.
- `src/commands/` — um módulo por subcomando:
  - `run/` — `config`, `engine` (orquestração + timing), `setup`, `live`
    (LiveTree + EventBus), `finalize`.
  - `serve/` — `mod` (execute + `ServeConfig`), `state`, `response`,
    `requests`, `handlers`, `search_logic`, `run_logic`, `index_logic`,
    `status_logic`.
  - `mcp/` — `protocol` (JSON-RPC), `session`, `handlers`.
  - `index`, `search`, `query`, `context`, `status`, `history`, `cost`,
    `session`, `consolidate`, `decay`, `cancel`, `checkpoints`,
    `restore_page`, `wiki`, `entities`, `persist`, `qa_cache` (plan 017:
    `run_ask`/`run_get`/`run_invalidate` orquestrando os RPCs `QueryWithCache`/
    `GetAnswerById`/`InvalidateCache`; digestão LLM roda localmente via
    `arlm-llm`/`config.toml` e o `StoreAnswer` é fire-and-forget).
- `src/cli/commands.rs` — `Commands` enum (inclui `Query` estendido com
  `cache_id`/`qa` e o subcomando `Cache { CacheCmd::Invalidate | Get }`).
- `src/dispatch/server.rs` — modo servidor: ramifica `Query` para
  `run_get`/`run_ask` e despacha `Cache` para `CacheCmd`.
- `src/dispatch/local.rs` — `Cache` retorna erro em modo local (é server-only).
- `src/output/` — `mod` (`Format`), `json`, `tree`, `markdown`, `prompt`,
  `live_tree/` (`model` + `render`).
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
- Padrão de produção: `dispatch::dispatch(cli, cfg, &rt)` resolve tudo e roteia.
- Modo servidor (`--server`) só suporta `search`, `status`, `session`, `run`,
  `cost`, `context` (o resto exige handlers gRPC em `arlm-server`/`arlm-proto`).
- `Config` é deserializável de TOML; a seção `[server].addr` alimenta o cliente.
- `--llm` é obrigatório em `run`; sem ele, `run::execute` retorna erro claro.
- `--persist` salva o output de `run`/`search`/`context` no wiki via
  `persist::save_page`.
