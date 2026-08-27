# arags-cli

Interface de linha de comando para o **arags** — *on-demand, agent-agnostic RLM*.
É um **cliente gRPC puro** que se conecta a um `arags-server` (plano de dados).
Usa o **LLM local do usuário** (`arags-llm`) apenas para *digest* (`query -qa`)
e *summarize* (`persist`); nenhuma outra operação depende de LLM.

## Responsabilidades

- **CLI (lib + bin):** `src/lib.rs` expõe a API pública; `src/main.rs` é um *thin binary*
  que faz o parsing e delega o dispatch.
- **Parsing:** `clap` derive em `src/cli/` (estrutura de subcomandos desacoplada de `main`).
- **Dispatch:** `src/dispatch/` resolve a config do usuário (2 escopos, local > global)
  e roteia para o servidor gRPC.
- **Comandos:** subcomandos (`commands/<cmd>`), incluindo o QA-Cache (plan 017) via
  `query --qa`/`--cache-id` e `memory invalidate`.
- **Output:** 4 formatos (`json`, `tree`, `markdown`, `prompt`) em `src/output/`.
- **Observabilidade:** logs estruturados via `tracing` (`--verbose`).
- **Resiliência de cliente:** retry com backoff, validação de endereço e TLS automático
  em `src/client.rs`.
- **Config do usuário (2 escopos):** `src/user_config.rs` lê `~/.arags/arags.toml`
  (global) e `.arags.toml` (local), com merge granular por campo. `[auth]` é só-global.
  Arquivos legados `config.toml` **não** são lidos.
- **Allocator:** mimalloc para performance.

## Estrutura

```
src/
├── lib.rs                 # API pública (re-exports)
├── main.rs                # Thin binary: parse → logging → dispatch
├── cli/                   # Definição dos argumentos (clap)
│   ├── mod.rs
│   ├── root.rs            # Cli, OutputFormatArg
│   └── commands.rs        # enum Commands
├── dispatch/              # Roteamento (único ponto que conhece os comandos)
│   ├── mod.rs             # resolve user_config/formato e roteia ao módulo certo
│   ├── index.rs           # index: descoberta + streaming zstd + paralelismo
│   ├── discover.rs        # regras de ignore (gitignore/default/user)
│   ├── projects.rs        # --register/--unregister do watch daemon
│   ├── watch_daemon.rs    # __watch: quiet-window re-index com fingerprint
│   ├── search.rs          # search/query + render multi-formato
│   ├── memory_history.rs  # memória/cache admin + histórico
│   └── init.rs            # scaffold .arags.toml (+ seed de .gitignore)
├── client.rs              # gRPC client: retry/backoff, TLS/mTLS, validação
├── auth_client.rs         # AuthRefresh + interceptor Bearer com renovação
├── backend.rs             # resolve o backend LLM do usuário ([llm.backends])
├── user_config/           # Config 2-escopos (global ~/.arags/arags.toml + local .arags.toml)
├── commands/              # módulos de comando
│   ├── mod.rs
│   ├── persist.rs         # wiki/*.md via LLM do usuário
│   └── qa_cache.rs        # plan 017: run_ask/run_get/run_invalidate
├── dispatch/
│   └── exploration.rs     # plan 022: arags explore (search/persist)
├── volunteer.rs           # `arags volunteer`: worker RLM com o LLM local
└── output/
    ├── mod.rs             # Format enum
    └── json.rs jsonl.rs tree.rs markdown.rs prompt.rs
tests/                     # testes de integração (user_config, init, output, ...)
```

## Comandos

| Comando | Descrição |
|---------|-----------|
| `arags init [--index] [--no-index]` | Scaffold de `<proj>/.arags.toml` (gitignored) + index |
| `arags index` | Faz stream do texto bruto; o servidor chunk+embed |
| `arags search` | Busca híbrida BM25 + semântica (server-side) |
| `arags query` | QA on-demand; `-qa` digere via LLM do usuário; `--cache-id` lookup; emite `cache_id` |
| `arags maintenance list\|get\|invalidate\|cleanup` | Manutenção do servidor (admin, via RPC) |
| `arags persist <response_id>` | Escreve `wiki/<yyyymmddhhmm>_<title>.md` (summarize via LLM do usuário) |
| `arags history [--limit] [--user]` | Histórico de consultas por usuário |

> **Removido (plan 020):** o subcomando `serve` (HTTP/MCP local) — o CLI é um
> cliente gRPC puro; quem hospeda o data plane é o binário `arags-server`.

> **Removidos (plan 019):** `run`, `context`, `session`, `status`, `cost`,
> `cancel`, `checkpoints`, `restore-page`, `wiki`, `consolidate` (CLI), `decay`
> (CLI) e `entities` (CLI). A manutenção server-side é feita por cron + RPC
> admin `TriggerMaintenance` (e `arags-server admin consolidate`).

## Flags Principais

### `arags index`

| Flag | Descrição | Default |
|------|-----------|---------|
| `--ignore <pattern>` | Padrões de ignore (glob) | `.env`, `*.pem`, `*.key` |

> O chunking e os embeddings ocorrem **no servidor**. O cliente faz stream do
> texto bruto (client-streaming gRPC `IndexProject`).

### `arags search`

| Flag | Descrição | Default |
|------|-----------|---------|
| `--top-k <N>` | Número de resultados | 10 |
| `--file-pattern <pat>` | Filtro por arquivo | — |
| `--min-score <f>` | Score mínimo | — |
| `--tier <t>` | `fts`, `entity`, `vector`, `hybrid`, `summary` ou `auto` | auto |

A resposta (plan 023) pode trazer três seções: **Results** (chunks),
**RLM Summaries** (sínteses aprovadas, até `[search].summary_ratio` do
budget) e **Exploration Maps** (refs com confidence) — renderizadas em todos
os formatos.

### `arags query`

| Flag | Descrição | Default |
|------|-----------|---------|
| `-qa` | Digere via LLM local do usuário (emite `cache_id`) | off |
| `--cache-id <id>` | Lookup determinístico 1:1 | — |

## Formatos de Saída

```bash
arags search "query" --format json       # JSON estruturado
arags search "query" --format tree       # Tabela colorida (default)
arags search "query" --format markdown   # Markdown
arags search "query" --format prompt      # Prompt para LLM
```

## Conexão com o Servidor (plan 020)

O alvo é resolvido na ordem: `.arags.toml` local `[server].addr` →
`~/.arags/arags.toml` global `[server].addr` → env `ARAGS_SERVER_ADDR` →
`127.0.0.1:50051`. Não existe flag `--server` (a config vive nos arquivos).

```toml
[server]
addr = "https://arags.corp.internal:50051"
tls_ca = "/etc/arags/tls/ca.crt"          # CA customizada (opcional)
tls_cert = "/etc/arags/tls/client.crt"    # mTLS: client cert (opcional,
tls_key = "/etc/arags/tls/client.key"     # exige também tls_key)
```

- Cliente com **retry/backoff** (3 tentativas), **validação de endereço** e
  **TLS automático** em `https://`; `tls_ca`/`tls_cert`/`tls_key` habilitam
  CA customizada e mTLS mesmo sem scheme.

## Flags Globais

```
--format <fmt>          # full_json|path|markdown|text|jsonl
--project <path>, -p    # escopo do projeto
--verbose, -v           # logs estruturados (tracing)
```

## Uso

```bash
# Inicializar + indexar
arags init ./meu-projeto

# Buscar com verbose
arags search "bug no login" --verbose

# QA com digest via LLM do usuário (emite cache_id)
arags query "analise auth" -qa
```

## Integração com Agentes

### OPencode
```json
{
  "name": "rlm_search",
  "command": "arags search \"{{task}}\" --format prompt"
}
```

### Cursor
```json
{
  "rlm": {
    "command": "arags search \"$ARGUMENTS\" --format prompt"
  }
}
```

## Build

```bash
cargo build -p arags-cli                 # Debug
cargo build --release -p arags-cli       # Release (otimizado)
# Binary: ./target/release/arags
```

## Testes

```bash
CARGO_BUILD_JOBS=4 cargo test -p arags-cli
```

Testes de integração ficam em `tests/` (incluindo `init_test.rs`, que valida o
scaffold do `arags init` e a ausência de dependências do data plane); testes
unitários puros vivem em `#[cfg(test)]` inline (ex.: merge da `user_config`).
