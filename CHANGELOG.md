# Changelog

Formato baseado em [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/).
Este projeto adere ao [Semantic Versioning](https://semver.org/lang/pt-BR/).

## [Unreleased]

### Added — ignore de dotfiles + `.gitignore` e auto-atualização com `index --register` (agnostic-rlm-rs-4442, 740a, fe41)

#### Ignore de arquivos (descoberta de arquivos)

- **Dot-paths ignorados**: todo caminho com qualquer componente iniciando
  por `.` (`.env`, `.git/`, `.github/workflows/x.yml`, ...) não é mais indexado.
  `--force-include` continua sobrescrevendo.
- **Regras de `.gitignore` respeitadas**: raiz e aninhados, com o subconjunto
  pragmático do git — comentários, dir-only (`logs/`), âncora (`/dist`),
  globs (`*`, `?`, `**`) e negação `!` (*last-match-wins*; arquivos mais
  profundos vencem os rasos). Novo módulo: `arags-cli/src/gitignore.rs`.

#### Watch daemon (`git maintenance`-style)

- **Novo flag** `arags index --register`: persiste o rastreamento no
  `.arags.toml` do projeto (`[watch] enabled = true` + nome do projeto,
  preservando os demais campos) e sobe um **daemon detached no client**
  (`arags watch-daemon <root>`).
- O daemon monitora a árvore via `notify`; cada mudança abre uma **janela de
  silêncio de 1 minuto**; ao fechá-la, re-envia **apenas os arquivos alterados**
  (fingerprint mtime+tamanho, respeitando todas as regras de ignore) ao
  servidor, que faz upsert dos chunks envolvidos.
- Controle sem sinais: marcadores dotfile `.arags-watch.pid` /
  `.arags-watch.stop`; `arags index --unregister` pede o stop gracioso e limpa
  a flag em `.arags.toml`.
- Novo módulo: `arags-cli/src/watcher.rs`.

### Removed — watch legado migrado (agnostic-rlm-rs-fe41)

- `arags-memory::watch` (`WatchMonitor`/`WatchHandle`/`WatchEvent`, do antigo
  experimento `--watch`), seus testes e a dependência `notify` do crate foram
  removidos; a funcionalidade de auto-atualização agora vive no client
  (`arags-cli/src/watcher.rs`, ver acima).

### Changed — embedding nativo all-MiniLM-L6-v2 (agnostic-rlm-rs-1194)

O modelo de embeddings virou **parte do projeto**: all-MiniLM-L6-v2 nativo em
candle (22M params, 384 dims, INT8 default), sem Ollama, sem Python, sem rede.

- **Backends alternativos removidos**: Ollama HTTP (imagem pesada) e BGE-M3
  (2,2 GB de weights) deletados; `[embedder].model` não existe mais.
- Config: `[embedder] model_dir` + `quantization = "int8"` + knobs de chunk.
- `VectorStore`/QA-cache defaults alinhados a 384 dims.
- **Reindex necessário** após atualizar.

### Removed — limpeza de código morto (pós planos 019/020)

Auditoria pós-consolidação removeu os resquícios que sobraram da arquitetura
antiga; o grafo do `arags-server` ficou **100% LLM-free** (nem transitive):

- **arags-search**: Tier 3 de LLM rerank (`rerank.rs`, `with_llm_backend`,
  `SearchTier::LlmRerank`) e a camada dual-layer da tabela `summaries`
  (`is_summary`/`summary_scope`); dependência `arags-llm` cortada.
- **proto**: RPCs de Session (`CreateSession`/`ListSessions`/`GetSession`/
  `AddSessionTurn`) + `session.proto`; campos/mensagens de summaries
  (`SummaryInfo`, `is_summary`, `include_summaries`, `total_summaries`,
  `SummarizeStatus`).
- **arags-server**: handlers/persistência de sessão, wrapper de summaries e
  contagem no status.
- **arags-storage**: módulo `sqlite/summaries.rs` + migrations
  006/012/014 (`sessions`, `summaries`, FTS5 de summaries).
- **arags-core**: placeholders `types/`, trait `MemoryProvider` e a dependência
  morta `arags-llm`.

### ⚠ BREAKING — plan 020 (consolidação de configuração)

Break **total, sem transição** (decisão D4 do plan 020): os arquivos legados
`~/.arags/config.toml` e `.arags/config.toml` são **ignorados** — não há fallback
nem aviso. Operadores devem reescrever suas configs nos novos arquivos:

| Arquivo novo | Quem lê | Conteúdo |
|---|---|---|
| `server.toml` (HOST; montado em `/etc/arags/server.toml` ou `ARAGS_SERVER_CONFIG`) | `arags-server` | todo o data plane: listen/TLS/mTLS, storage (`pool_size`, `flush_interval_ms`, `max_batch_size`), `[embedder]` (chunk+embed), `[search]`, `[qa_cache]`, `[maintenance]`, `[history] retention_days` |
| `~/.arags/arags.toml` (global) | `arags-cli` | `[auth]` (só global) + `[llm.backends]` + `[server]` (`addr`, `tls_ca`, `tls_cert`, `tls_key`) |
| `.arags.toml` (local, gitignored via `arags init`) | `arags-cli` | overrides por projeto + `[project]`; `[auth]` local é ignorado |

Mudanças de superfície relacionadas:

- **Modo offline removido (D3).** O `arags-cli` é um puro gRPC client: os
  comandos `serve`/`--mcp` locais foram deletados. Quem quiser "offline" sobe
  o próprio `arags-server`.
- **Server faz o chunking (D2).** O client envia texto cru; o tamanho de chunk
  vem de `[embedder].max_tokens/overlap_tokens`. Reindex necessário.
- **`[search].tier` default do server**: o proto `SearchTier` ganhou
  `SEARCH_TIER_UNSPECIFIED = 0` (valores explícitos renumerados 1–4); requests
  sem tier resolvem para o default de `server.toml`.
- **Embedder configurável só no server**: variáveis
  `ARAGS_MODEL_DIR`/`ARAGS_OLLAMA_*`/`ARAGS_EMBED_BATCH` foram substituídas por
  `[embedder]` no `server.toml` (`ARAGS_SERVER_ADDR`/`ARAGS_DATA_DIR` continuam
  como overrides de env).

## [0.1.0]

### Added

- Workspace inicial (9 crates): CLI gRPC, server data plane (gRPC/TLS),
  storage SQLite/LanceDB, embeddings BGE-M3/Ollama/lightweight, busca híbrida
  BM25+semântica+RRF, QA-cache semântico (plan 017), auth por refresh token
  (plan 018), memória multi-projeto.
