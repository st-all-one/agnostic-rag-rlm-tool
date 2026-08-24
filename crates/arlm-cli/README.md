# arlm-cli

Interface de linha de comando para o **arlm** — *on-demand, agent-agnostic RLM*.
É um **cliente gRPC puro** que se conecta a um `arlm-server` (plano de dados).
Usa o **LLM local do usuário** (`arlm-llm`) apenas para *digest* (`query -qa`)
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
- **Config do usuário (2 escopos):** `src/user_config.rs` lê `~/.arlm/arlm.toml`
  (global) e `.arlm.toml` (local), com merge granular por campo. `[auth]` é só-global.
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
├── dispatch/              # Roteamento de comandos
│   ├── mod.rs             # branch para o servidor gRPC
│   └── server.rs          # modo servidor gRPC (formatado por --format)
├── client.rs              # gRPC client: retry/backoff, TLS, validação
├── user_config.rs         # Config 2-escopos (global ~/.arlm/arlm.toml + local .arlm.toml)
├── util.rs                # data_dir(), project resolution
├── commands/              # um módulo por subcomando
│   ├── mod.rs
│   ├── index.rs  search.rs  query.rs  qa_cache.rs
│   ├── memory.rs  persist.rs  history.rs
│   └── serve/             # arlm server (gRPC/MCP data plane)
└── output/
    ├── mod.rs             # Format enum
    └── json.rs tree.rs markdown.rs prompt.rs
tests/                     # testes de integração
```

## Comandos

| Comando | Descrição |
|---------|-----------|
| `arlm init [--index] [--no-index]` | Scaffold de `<proj>/.arlm.toml` (gitignored) + index |
| `arlm index` | Faz stream do texto bruto; o servidor chunk+embed |
| `arlm search` | Busca híbrida BM25 + semântica (server-side) |
| `arlm query` | QA on-demand; `-qa` digere via LLM do usuário; `--cache-id` lookup; emite `cache_id` |
| `arlm memory list\|get\|invalidate\|cleanup` | Memória (admin, via RPC) |
| `arlm persist <response_id>` | Escreve `wiki/<yyyymmddhhmm>_<title>.md` (summarize via LLM do usuário) |
| `arlm history [--limit] [--user]` | Histórico de consultas por usuário |
| `arlm server` | Hospeda o servidor gRPC/MCP (plano de dados, sem `/run`) |

> **Removidos (plan 019):** `run`, `context`, `session`, `status`, `cost`,
> `cancel`, `checkpoints`, `restore-page`, `wiki`, `consolidate` (CLI), `decay`
> (CLI) e `entities` (CLI). A manutenção server-side é feita por cron + RPC
> admin `TriggerMaintenance` (e `arlm-server admin consolidate`).

## Flags Principais

### `arlm index`

| Flag | Descrição | Default |
|------|-----------|---------|
| `--ignore <pattern>` | Padrões de ignore (glob) | `.env`, `*.pem`, `*.key` |

> O chunking e os embeddings ocorrem **no servidor**. O cliente faz stream do
> texto bruto (client-streaming gRPC `IndexProject`).

### `arlm search`

| Flag | Descrição | Default |
|------|-----------|---------|
| `--top-k <N>` | Número de resultados | 10 |
| `--file-pattern <pat>` | Filtro por arquivo | — |
| `--min-score <f>` | Score mínimo | — |

### `arlm query`

| Flag | Descrição | Default |
|------|-----------|---------|
| `-qa` | Digere via LLM local do usuário (emite `cache_id`) | off |
| `--cache-id <id>` | Lookup determinístico 1:1 | — |

## Formatos de Saída

```bash
arlm search "query" --format json       # JSON estruturado
arlm search "query" --format tree       # Tabela colorida (default)
arlm search "query" --format markdown   # Markdown
arlm search "query" --format prompt      # Prompt para LLM
```

## Modo Servidor (`--server`)

```bash
arlm --server 127.0.0.1:50051 search "query"
arlm --server 127.0.0.1:50051 query "como funciona o login?" -qa
```

- O endereço padrão é lido da seção `[server]` do `~/.arlm/arlm.toml` (global) ou
  `.arlm.toml` (local, campo `addr`), depois da env `ARLM_SERVER_ADDR`.
- Cliente com **retry/backoff** (3 tentativas), **validação de endereço** e
  **TLS automático** quando a URL usa `https://`.

## Flags Globais

```
--format <fmt>          # json|tree|markdown|prompt
--server <addr>         # usa gRPC remoto
--verbose, -v           # logs estruturados (tracing)
```

## Uso

```bash
# Inicializar + indexar
arlm init ./meu-projeto

# Buscar com verbose
arlm search "bug no login" --verbose

# QA com digest via LLM do usuário (emite cache_id)
arlm query "analise auth" -qa

# Servidor remoto
arlm --server 127.0.0.1:50051 search "query"
```

## Integração com Agentes

### OPencode
```json
{
  "name": "rlm_search",
  "command": "arlm search \"{{task}}\" --format prompt"
}
```

### Cursor
```json
{
  "rlm": {
    "command": "arlm search \"$ARGUMENTS\" --format prompt"
  }
}
```

## Build

```bash
cargo build -p arlm-cli                 # Debug
cargo build --release -p arlm-cli       # Release (otimizado)
# Binary: ./target/release/arlm
```

## Testes

```bash
CARGO_BUILD_JOBS=4 cargo test -p arlm-cli
```

Testes de integração ficam em `tests/`; não há `#[cfg(test)]` dentro de `src/`.
