# arags-cli

## O que faz
CLI *agent-agnostic* do `arags`: faz o parsing de argumentos (clap), resolve a
configuração do usuário (**2 escopos**: `~/.arags/arags.toml` global +
`.arags.toml` local, com merge granular por campo) e roteia cada subcomando para
um `arags-server` remoto via gRPC. É um **cliente gRPC puro**: não há modo local
(plan 020 removeu o subcomando `serve`/MCP e o data plane local). Usa o **LLM local do usuário** (`arags-llm`) apenas para *digest*
(`query -qa`) e *summarize* (`persist`). O servidor é um plano de dados puro
(LLM-free). Renderiza saídas em 4 formatos (`json`, `tree`, `markdown`, `prompt`)
com logs estruturados (`tracing`). Também hospeda o **watch daemon** de
auto-atualização do índice (`arags index --register`), o único processo de
longa duração do client — ainda assim, sem estado local além dos marcadores
dotfile e do registro em `.arags.toml`.

## Estrutura
- `src/lib.rs` — API pública (re-exports) + allows de lint (pedantic estilo).
- `src/main.rs` — *thin binary*: parse → `init_logging` → `dispatch`.
- `src/cli/` — `Cli`, `Commands`, `OutputFormatArg` (clap derive, desacoplado
  do entry point).
- `src/dispatch/` — **plan 021 (ex-`server.rs` de 1116 linhas), dividido por
  responsabilidade:** `mod.rs` (`dispatch()` resolve config/formato e roteia;
  `connect()` com AuthRefresh; `map_search_tier`), `index.rs` (`run_index` +
  `stream_index_group` — **upload zstd-comprimido, level 3** — e
  `partition_files`), `discover.rs` (descoberta de arquivos; ignore rules via
  comparação por componente, sem `format!` no hot loop),
  `projects.rs` (--register/--unregister), `watch_daemon.rs`
  (`__watch`: quiet-window re-index com fingerprint mtime+size e
  `WATCH_UPLOAD_PARALLELISM = 2`), `search.rs` (search/query + render multi-formato),
  `memory_history.rs` (memória/cache admin + histórico) e `init.rs`
  (scaffold `.arags.toml`, seed do `.gitignore`). Não há modo local — todo
  comando vai para o servidor. Testes por módulo em `<name>/tests.rs`.
- `src/auth_client.rs` — `AragsClient` autenticado (`AuthRefresh` + interceptor
  Bearer com renovação em background).
- `src/backend.rs` — resolve o backend LLM do usuário a partir de
  `[[llm.backends]]` (usado por `query -qa` e `persist`).
- `src/client.rs` — `ClientConfig` + `connect_channel` (retry/backoff, validação
  de endereço, TLS automático em `https://` e mTLS via `[server].tls_ca`/
  `tls_cert`/`tls_key`).
- `src/gitignore.rs` — parser do subconjunto gitignore usado na descoberta de
  arquivos (`*`/`?`/`**`, dir-only, âncora `/`, negação `!` com
  *last-match-wins*, precedência por profundidade para `.gitignore`
  aninhados). Funções puras; testes em `gitignore/tests.rs`.
- `src/watcher.rs` — auto-atualização estilo `git maintenance`
  (`arags index --register`): daemon detached (`arags watch-daemon <root>`)
  via `notify`, **janela de silêncio de 1 min** e flush só dos arquivos
  alterados (fingerprint mtime+size); registro em `[watch]` no `.arags.toml`;
  controle por marcadores dotfile (`.arags-watch.pid`/`.arags-watch.stop`),
  sem sinais nem `unsafe`.
- `src/user_config/` — **plan 021 (ex-`user_config.rs`):** `mod.rs` com os
  tipos (`AuthConfig`, `ServerSection`, `ProjectSection`, `WatchSection`,
  `GlobalConfig`, `VolunteerConfig`, `LocalConfig`, `EffectiveUserConfig`) e
  `ops.rs` com load/merge (`load`, `load_from`, `merge`, `merge_backends`,
  `resolve_addr`, `set_watch_enabled`). Config 2-escopos (`[auth]` só-global,
  `[llm]`, `[server]` com knobs TLS, `[project]`, `[watch]` local-only); merge
  granular por campo; arquivos legados `config.toml` **não** são lidos.
  Testes: `tests/user_config_test.rs`.
- `src/commands/` — módulos de comando:
  - `qa_cache` (plan 017: `run_ask`/`run_get`/`run_invalidate` orquestrando os
    RPCs `QueryWithCache`/`GetAnswerById`/`InvalidateCache`; a digestão LLM roda
    localmente via `arags-llm`/`user_config` e o `StoreAnswer` é fire-and-forget),
  - `persist` (escreve `wiki/*.md` via LLM do usuário).
  - `index`, `search`, `query`, `memory` (admin), `history` vivem nos módulos
    de `dispatch/` (plan 021): `index.rs` (streaming de arquivos comprimidos),
    `search.rs`, `memory_history.rs` e `init.rs`; o watch daemon está em
    `watch_daemon.rs` e o registro em `projects.rs`.
- `src/cli/commands.rs` — `Commands` enum (inclui `Query` estendido com
  `cache_id`/`qa` e o subcomando `Memory`).
- `src/output/` — `mod` (`Format`), `json`, `tree`, `markdown`, `prompt`.
- `tests/` — testes de integração (um arquivo por módulo); sem `#[cfg(test)]`
  em `src/`.

## Dependências
- Internas: `arags-core`, `arags-llm`, `arags-proto` (plan 020: sem
  `arags-storage`/`arags-search`/`arags-memory` — o client nunca abre estado local;
  guardado por teste em `tests/init_test.rs`).
- Externas: `clap` (derive), `tokio`/`tokio-stream` (async/streaming),
  `tonic` (gRPC), `notify` (watch daemon do `--register`),
  `tracing`/`tracing-subscriber` (logs), `serde`/`serde_json`/
  `toml` (config/saída), `anyhow` (erros), `indicatif`/`console` (UI),
  `chrono` (timestamps do wiki), `parking_lot` (sync), `mimalloc` (allocator),
  `zstd` (compressão do upload de indexação, plan 021).

## Módulos RLM do cliente
- `src/dispatch/exploration.rs` — **plan 022:** `arags explore {search,persist,
  feedback}`; parser puro do contrato EXPLORATIONS.md (`parse_contract`) com
  testes em `dispatch/exploration/tests.rs`.
- `src/volunteer.rs` — **worker voluntário (`arags volunteer`)**: loop claim →
  síntese com o LLM local → submit. Helpers puras testáveis: `parse_inputs`
  (payload vazio/malformado rejeitado), `build_request`/`system_prompt_for`
  (templates L1/L2/L3, temperatura 0.2) e `summary_acceptable`
  (`MIN_SUMMARY_CHARS = 20`). Usa `RlmJobPayload`/`DEFAULT_RLM_LEASE_MS` de
  `arags_core::rlm`. Testes em `volunteer/tests.rs`.

## Convenções deste módulo
- Sem `unwrap`/`expect`/`panic`/`unsafe` em `src/`; use `anyhow` + `?`.
- Logs estruturados via `tracing` (`info!`/`debug!`/`warn!` com campos) e
  *timing* via `std::time::Instant` registrado como `elapsed_ms`.
- Segurança de thread: estado compartilhado é `Send + Sync`
  (`Arc` + `parking_lot::Mutex`/`RwLock`).
- Performance: evitar clones desnecessários; `with_capacity` quando o tamanho
  é conhecido.
- Testes fora do corpo dos fontes (plan 021): suítes de API pública em
  `tests/*_test.rs` e unitários de módulo em `<name>/tests.rs`; o gate
  `scripts/check_file_length.sh` mantém todo `src/*.rs` ≤300 linhas de
  produção.
- `dispatch` é o único ponto que conhece a árvore de comandos; `commands::*`
  expõe `execute(Config)` estável.
- Testes de API pública ficam em `tests/`; funções puras críticas (merge da
  config) têm `#[cfg(test)]` inline com tempdirs.

## Comandos úteis
```bash
CARGO_BUILD_JOBS=4 cargo check   -p arags-cli --all-targets
CARGO_BUILD_JOBS=4 cargo clippy   -p arags-cli --all-targets -- -D warnings
CARGO_BUILD_JOBS=4 cargo test     -p arags-cli
cargo fmt -p arags-cli -- --check
```

## Migrations
- N/A — o crate não possui schema próprio (estado em `arags-storage`/`arags-memory`).

## Rules
- Padrão de produção: `dispatch::dispatch(cli, &rt)` carrega a user_config e
  roteia tudo para o servidor gRPC; nenhum comando abre Storage local.
- O CLI é um **cliente gRPC puro**: todos os comandos (`init`, `index`,
  `search`, `query`, `memory`, `persist`, `history`, `server`) vão para o
  `arags-server` (plano de dados, LLM-free).
- `--llm` NÃO existe no CLI: o LLM do usuário é usado implicitamente em
  `query -qa` (digest) e `persist` (summarize) via `arags-llm`.
- A config do usuário é 2-escopos (`user_config.rs`): `~/.arags/arags.toml`
  (global, com `[auth]` só-global) + `.arags.toml` (local, merge por campo,
  `[project]`). `server.addr` alimenta o cliente.
