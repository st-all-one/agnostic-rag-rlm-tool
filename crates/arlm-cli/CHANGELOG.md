# Changelog

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
- `ARLM_DATA_DIR` env var para sobrescrever diretório de dados (testes)

### Changed
- **BREAKING:** Todos os comandos agora usam DB compartilhado (`~/.arlm/knowledge.db`)
- `project_dirs()` renomeado para `data_dir()` (retorna `~/.arlm/`)
- Helper `project_name()` extraído para `util.rs`

## [0.1.0] - 2026-08-19

### Added
- 10 subcomandos: run, index, search, query, context, status, history, cost, session, consolidate, serve
- 4 formatos de output: JSON, Tree, Markdown, Prompt
- mimalloc allocator para performance
- Flag --verbose para logs estruturados
- Integração com arlm-core::ScopedTimer para profiling
- Output formatado com cores (console crate)
- Progress bars (indicatif crate)
- Unit tests (20 testes)
