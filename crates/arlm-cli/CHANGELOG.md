# Changelog

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
