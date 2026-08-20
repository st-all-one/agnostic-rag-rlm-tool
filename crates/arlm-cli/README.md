# arlm-cli

Interface de linha de comando para o **arlm** — *agent-agnostic Recursive Language Model*.
Conecta-se a um `arlm-server` via gRPC (`--server`) ou roda localmente.

## Responsabilidades

- **CLI (lib + bin):** `src/lib.rs` expõe a API pública; `src/main.rs` é um *thin binary*
  que faz o parsing e delega o dispatch.
- **Parsing:** `clap` derive em `src/cli/` (estrutura de subcomandos desacoplada de `main`).
- **Dispatch:** `src/dispatch/` resolve a precedência de config (CLI > config > ...)
  e roteia para modo local ou servidor gRPC.
- **Comandos:** 19 subcomandos, um módulo `commands/<cmd>` cada (alguns subdivididos).
- **Output:** 4 formatos (`json`, `tree`, `markdown`, `prompt`) em `src/output/`.
- **Observabilidade:** logs estruturados via `tracing` (`--verbose`) e *timing* de fases
  com `std::time::Instant` (registrado como `elapsed_ms`).
- **Resiliência de cliente:** retry com backoff, validação de endereço e TLS automático
  em `src/client.rs`.
- **Allocator:** mimalloc para performance.

## Estrutura

```
src/
├── lib.rs                 # API pública (re-exports) + allows de lint
├── main.rs                # Thin binary: parse → logging → dispatch
├── cli/                   # Definição dos argumentos (clap)
│   ├── mod.rs
│   ├── root.rs            # Cli, OutputFormatArg, parse_tool_arg
│   └── commands.rs        # enum Commands + SessionAction
├── dispatch/              # Roteamento de comandos
│   ├── mod.rs             # resolução de config + branch local/servidor
│   ├── local.rs           # execução local (chama commands::*)
│   └── server.rs          # modo servidor gRPC (formatado por --format)
├── client.rs              # gRPC client: retry/backoff, TLS, validação
├── config.rs              # Config (TOML) + seção [server]
├── metrics.rs             # ArlmMetrics (Prometheus)
├── util.rs                # data_dir(), project_name()
├── commands/              # um módulo por subcomando
│   ├── mod.rs
│   ├── run/               # engine, setup, live (LiveTree), finalize
│   ├── serve/             # HTTP/MCP server (handlers, state, logic)
│   ├── mcp/               # MCP protocol (protocol, session, handlers)
│   ├── index.rs  search.rs  query.rs  context.rs
│   ├── status.rs history.rs cost.rs session.rs
│   ├── consolidate.rs decay.rs cancel.rs checkpoints.rs
│   ├── restore_page.rs wiki.rs entities.rs persist.rs
└── output/
    ├── mod.rs             # Format enum
    ├── json.rs tree.rs markdown.rs prompt.rs
    └── live_tree/         # LiveTree (model + render) para --live
tests/                     # testes de integração (sem #[cfg(test)] em src/)
```

## Comandos

| Comando | Descrição | Requer `--llm` |
|---------|-----------|---------------|
| `arlm index` | Indexa projeto | Não |
| `arlm search` | Busca híbrida | Não |
| `arlm context` | Contexto para agente | Não |
| `arlm query` | Consulta com RLM | Sim |
| `arlm run` | Executa RLM recursivo | **Sim** (`--llm` obrigatório) |
| `arlm status` | Mostra projetos indexados | Não |
| `arlm history` | Histórico de consultas | Não |
| `arlm cost` | Resumo de custos | Não |
| `arlm session` | Gerencia sessões | Não |
| `arlm consolidate` | Limpa memória | Não |
| `arlm persist` | Salva como wiki pages | Não |
| `arlm decay` | Salience decay | Não |
| `arlm serve` | HTTP/MCP server | Não |
| `arlm cancel` | Cancela run | Não |
| `arlm checkpoints` | Lista checkpoints | Não |
| `arlm restore-page` | Restaura wiki page | Não |
| `arlm wiki` | Gerencia wiki (git) | Não |
| `arlm entities` | Busca entidades | Não |
| `arlm mcp` | Model Context Protocol | Não |

## Flags Principais

### `arlm run`

```bash
arlm run "analise completa" --llm --backend openai
arlm run "refactor" --llm --backend anthropic --model claude-3.5-sonnet
arlm run "fix bug" --llm --depth 5 --max-nodes 100 --live
arlm run "doc" --llm --persist          # salva o output no wiki
```

| Flag | Descrição | Default |
|------|-----------|---------|
| `--llm` | **Obrigatório** para RLM | — |
| `--backend <name>` | openai, anthropic, ollama, gemini, deepseek, mimo | ollama |
| `--model <name>` | Modelo | — |
| `--depth <N>` | Profundidade recursão | 3 |
| `--max-nodes <N>` | Máximo de nós | 50 |
| `--concurrency <N>` | Concorrência | 4 |
| `--max-budget <USD>` | Orçamento | 1.0 |
| `--live` | Árvore em tempo real (LiveTree) | off |
| `--persist` | Salva o resultado como wiki page | off |
| `--session <id>` | Sessão multi-turno | — |
| `--tool name:desc` | Tool custom p/ o solver | — |

### `arlm search` / `arlm context`

```bash
arlm search "auth middleware" --all --tier entity --max-tokens 4000
arlm context "fix login bug" --persist
```

| Flag | Descrição | Default |
|------|-----------|---------|
| `--top-k <N>` | Número de resultados | 10 |
| `--all` / `-a` | Busca cross-project | off |
| `--tier <tier>` | `fts`, `entity`, `vector`, `auto` | auto |
| `--max-tokens <N>` | Limite de tokens (0=ilimitado) | 8000 |
| `--file-pattern <pat>` | Filtro por arquivo | — |
| `--min-score <f>` | Score mínimo | — |
| `--persist` | Salva o resultado como wiki page | off |

## Formatos de Saída

```bash
arlm search "query" --format json       # JSON estruturado
arlm search "query" --format tree       # Tabela colorida (default)
arlm search "query" --format markdown   # Markdown
arlm search "query" --format prompt     # Prompt para LLM
```

O `--format` também é respeitado no modo servidor (`--server`): as respostas gRPC
são renderizadas conforme o formato escolhido.

## Modo Servidor (`--server`)

```bash
arlm --server 127.0.0.1:50051 search "query"
arlm --server 127.0.0.1:50051 status
```

- Suporta `search`, `status`, `session`, `run`, `cost`, `context`.
- O endereço padrão é lido da seção `[server]` do `~/.arlm/config.toml` ou
  `.arlm/config.toml` (campo `addr`), depois da env `ARLM_SERVER_ADDR`.
- Cliente com **retry/backoff** (3 tentativas), **validação de endereço** e
  **TLS automático** quando a URL usa `https://`.

## Flags Globais

```
--project <path>        # Caminho do projeto (default: .)
--format <fmt>          # json|tree|markdown|prompt
--config <path>         # arquivo de config (default: ~/.arlm/config.toml)
--backend <name>        # override de backend
--model <name>          # override de modelo
--agent <name>          # override de agente
--server <addr>         # usa gRPC remoto em vez de local
--verbose, -v           # logs estruturados (tracing)
```

## Uso

```bash
# Indexar projeto
arlm index ./meu-projeto

# Buscar com verbose (logs estruturados + timing)
arlm search "bug no login" --verbose

# Contexto formatado
arlm context "analise auth" --format prompt

# Com LLM (--llm obrigatório)
arlm run "analise completa" --llm --backend openai

# Servidor remoto
arlm --server 127.0.0.1:50051 status
```

## Integração com Agentes

### OPencode
```json
{
  "name": "rlm_context",
  "command": "arlm context \"{{task}}\" --project {{cwd}} --format prompt"
}
```

### Cursor
```json
{
  "rlm": {
    "command": "arlm context \"$ARGUMENTS\" --project . --format prompt"
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

Testes de integração ficam em `tests/` (um arquivo por módulo); não há `#[cfg(test)]`
dentro de `src/`. Use sempre `CARGO_BUILD_JOBS=4`.
