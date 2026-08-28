# Changelog

## [Unreleased]

### Added — plan 023: render da Unified Contextual Query

- `arags search` (e `query`) renderizam as novas seções aditivas do proto:
  **"RLM Summaries"** (`SearchResponse.summaries` — nível, assunto, score e
  trecho) e **"Exploration Maps"** (`SearchResponse.explorations` — goal,
  confidence e summary) nos quatro formatos (`text`/`markdown`/`json`/
  `jsonl`). Clientes/scripts que só leem `results` seguem intactos.

### Added — plan 022: comando `arags explore`
- **`explore search "<query>"`** (`--project/--limit/--include-stale`): busca
  semântica de mapas persistidos; render text/json com status, confidence e
  stale_reason granular.
- **`explore persist --map FILE|-`**: valida o contrato EXPLORATIONS.md
  localmente (`parse_contract`: header goal/files obrigatórios + seções
  Mapa/Conexões/Evidências/Limitações; summary derivado do primeiro parágrafo
  do Mapa), dedup de `--paths` extras, stdin suportado.
- **`explore`**: subcomando de feedback do consumidor removido depois
  (risco sybil por IA; ver `agnostic-rag-rlm-tool-f5f3`) — permaneceram apenas
  `search`/`persist`.
- 5 testes novos (parser happy/rejeições/tolerante + rendering).

### Changed (plan 021 — split do dispatch e upload comprimido)
- **`dispatch/server.rs` (1116 linhas) dividido por responsabilidade** em
  `dispatch/{mod,index,discover,projects,watch_daemon,search,memory_history,init}.rs`
  — nenhum arquivo passa de 300 linhas de produção; o dispatcher (`dispatch()`)
  resolve config/formato e cada módulo roteia seu comando.
- **Upload zstd-comprimido:** `stream_index_group` envia o conteúdo dos
  arquivos comprimidos (zstd level 3, `compressed = true`) — o servidor já
  decodificava transparentemente; cai o tráfego de rede na indexação. Fallback
  para bytes crus (flag correta) se o encode falhar.
- **Discovery sem alocação no hot loop:** `is_default_ignored`/`matches_pattern`
  reescritos com comparação por componente de caminho (`has_component`) em vez
  de `format!` por arquivo×padrão; semântica original preservada (testes de
  tabela cobrem `dir/`, `*.ext`, `*sub*`, exato, caminho parcial).
- Paralelismo do watch daemon documentado como const
  (`WATCH_UPLOAD_PARALLELISM = 2`).

### Changed (plan 021 — user_config e volunteer)
- `user_config.rs` dividido em `user_config/{mod.rs,ops.rs}` (tipos vs.
  load/merge/watch); `resolve_addr` e os campos de `LocalConfig` são `pub`
  (merge granular testável de fora do crate).
- `volunteer.rs` ganhou helpers puras testáveis — `parse_inputs` (valida payload
  vazio/malformado com contexto) e `summary_acceptable` (gate de summary curta,
  `MIN_SUMMARY_CHARS = 20`) — e consome `RlmJobPayload`/`DEFAULT_RLM_LEASE_MS`
  de `arags_core::rlm` (fim das cópias locais).
- Testes inline extraídos: `tests/user_config_test.rs` (11 — merge granular,
  auth global-only, legado ignorado, round-trip do watch flag) e submódulos
  `volunteer/tests.rs` (5), `dispatch/*/tests.rs`, `watch_daemon/tests.rs`
  (4), `gitignore/tests.rs`, `watcher/tests.rs`.

### Added
- Dependência `zstd` (compressão do upload de indexação).

### Fixed
- `arags init` não carimba mais `[server] addr = "127.0.0.1"` hardcoded no
  `.arags.toml` local (agnostic-rag-rlm-tool-152a) — o addr do `~/.arags/arags.toml`
  global passa a valer; default continua `127.0.0.1:50051` quando nada é
  configurado.

> **Nota (planos 019/020):** o CLI passou por uma consolidação. Foram **removidos**
> os subcomandos `run`, `context`, `session`, `status`, `cost`, `cancel`,
> `checkpoints`, `restore-page`, `wiki`, `consolidate`, `decay` e `entities`, e o
> modo local — o `arags-cli` é agora um **cliente gRPC puro**. O servidor
> (`arags-server`) é um **plano de dados LLM-free**; o LLM do usuário é usado
> apenas em `query -qa` (digest) e `persist` (summarize). A config passou a ser
> 2-escopos (`~/.arags/arags.toml` global + `.arags.toml` local; `[auth]` só-global);
> `config.toml` legado não é lido. Veja `plan/019-cli-consolidation.md` e
> `plan/020-config-consolidation.md`.

### Changed / Removed (auditoria plan 020)
- **Removido o subcomando `serve`** (HTTP/MCP local) e todo o resto do data
  plane local: `commands/serve/`, `commands/mcp/`, `metrics.rs` e `util::data_dir`
  — o CLI é um **cliente gRPC puro** e não depende mais de `arags-storage`,
  `arags-search`, `arags-memory`, `axum` nem `tower-http`.
- **mTLS no cliente:** `[server] tls_ca`/`tls_cert`/`tls_key` na user config
  (merge granular global→local) alimentam `ClientConfig` (`client.rs`).
- Endereço resolvido apenas por `.arags.toml` → `~/.arags/arags.toml` → env
  `ARAGS_SERVER_ADDR` (a flag inexistente `--server` saiu da documentação).

### Added (auditoria plan 020)
- Testes inline da `user_config`: merge granular/recursivo, `[auth]` só-global,
  legados ignorados, precedência de endereço, campos TLS.
- `tests/init_test.rs`: scaffold do `arags init` (`.arags.toml` gitignored,
  sem credenciais locais) e guarda contra dependências de data plane.

### Added
- **QA-Cache client (plan 017):** `commands/qa_cache.rs` com `run_ask` (usa
  `QueryWithCache`; em HIT devolve a resposta sem chamar LLM; em MISS sintetiza
  top-K com o LLM do usuário via `arags-llm`/`config.toml`, exibe e dispara
  `StoreAnswer` fire-and-forget), `run_get` (`GetAnswerById` por `cache_id`) e
  `run_invalidate` (`InvalidateCache` Stale/Delete + raio).
- `cli/commands.rs`: `Query` estendido com `--qa`/`--cache-id` e subcomando
  `Cache { Invalidate | Get }`; `dispatch/server.rs` roteia para `qa_cache`.
- Auth implícita: o cliente anexa `Authorization: Bearer <session>` obtido via
  `AuthRefresh` (plan 018) nas chamadas gRPC que exigem autenticação.

## [0.3.0] - 2026-08-20

### Added
- Reorganização em **lib + bin**: `src/lib.rs` expõe a API pública; `src/main.rs`
  é um *thin binary* que faz parse e delega ao `dispatch`.
- Módulo `cli/` desacoplado: definição dos argumentos (`Cli`, `Commands`,
  `SessionAction`, `parse_tool_arg`) separada do entry point.
- Módulo `dispatch/` (`mod`/`local`/`server`) com resolução de precedência de
  config e roteamento local/servidor.
- `commands/run/`, `commands/serve/`, `commands/mcp/` e `output/live_tree/`
  divididos em módulos menores (<300 linhas), type-driven, com logs
  estruturados (`tracing`) e *timing* de fases (`elapsed_ms`).
- Testes de `#[cfg(test)]` extraídos de `src/` para `tests/` (26 arquivos de
  integração); `src/` não contém mais testes inline.
- `--persist` em `run`, `search` e `context` (salva output no wiki).
- `--llm` obrigatório em `run` (erro claro sem a flag).
- Cliente gRPC resiliente: retry com backoff exponencial, validação de
  endereço e TLS automático (`https://`).
- Seção `[server]` no `config.toml` (`addr`) lida pelo `ClientConfig::load()`.
- `--format` respeitado também no modo servidor (`--server`).
- Mapeamento `--tier` → `SearchTier` do proto (fts/entity/vector/auto).
- `LiveTree` integrado ao `run --live` via EventBus.

### Changed
- `run`, `serve`, `mcp` e `live_tree` refatorados em módulos menores com
  observabilidade (logs + timing) e segurança de thread (`Send + Sync`).
- Mensagem de erro do modo servidor lista os comandos suportados.

## [0.2.0] - 2026-08-19

### Added
- Flag `--all` / `-a` em `search` e `context` para busca cross-project
- Flag `--ignore` em `index` para padrões de ignore (glob). Default: `.env`, `.env.*`, `*.pem`, `*.key`
- Flag `--tier` em `search` e `context`: `fts`, `entity`, `vector`, `auto` (default: `auto`)
- Flag `--max-tokens` em `search` e `context` (default: 8000, 0=ilimitado)
- Flag `--watch` / `-w` em `index` para reindexação automática a cada mudança
- `search` e `context` agora são async (tokio) para suportar tiers híbridos
- `EntitySearch` integrado ao CLI (regex-based, sem dependência de embeddings)
- `ARAGS_DATA_DIR` env var para sobrescrever diretório de dados (testes)

### Changed
- **BREAKING:** Todos os comandos agora usam DB compartilhado (`~/.arags/knowledge.db`)
- `project_dirs()` renomeado para `data_dir()` (retorna `~/.arags/`)
- Helper `project_name()` extraído para `util.rs`

## [0.1.0] - 2026-08-19

### Added
- 10 subcomandos: run, index, search, query, context, status, history, cost, session, consolidate, serve
- 4 formatos de output: JSON, Tree, Markdown, Prompt
- mimalloc allocator para performance
- Flag --verbose para logs estruturados
- Integração com arags-core::ScopedTimer para profiling
- Output formatado com cores (console crate)
- Progress bars (indicatif crate)
- Unit tests (20 testes)
