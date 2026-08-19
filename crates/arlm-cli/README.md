# arlm-cli

Interface de linha de comando para o arlm — agent-agnostic.

## Responsabilidades

- **CLI**: Parsing de argumentos com clap derive
- **Comandos**: 11 subcomandos para todas as operações
- **Output**: 4 formatos (JSON, Tree, Markdown, Prompt)
- **Allocator**: mimalloc para performance
- **Logging**: Flag `--verbose` para logs estruturados
- **Async**: search e context são async (tokio) para tiers híbridos

## Estrutura

```
src/
├── main.rs              # Entry point, mimalloc, clap, tokio runtime
├── util.rs              # data_dir(), project_name() helpers
├── commands/
│   ├── mod.rs
│   ├── run.rs           # arlm run "tarefa" (--llm)
│   ├── index.rs         # arlm index ./projeto (--watch, --ignore)
│   ├── search.rs        # arlm search "query" (--all, --tier, --max-tokens)
│   ├── query.rs         # arlm query "pergunta"
│   ├── context.rs       # arlm context "tarefa" (--all, --tier, --max-tokens)
│   ├── status.rs        # arlm status
│   ├── history.rs       # arlm history
│   ├── cost.rs          # arlm cost
│   ├── session.rs       # arlm session create/resume
│   ├── consolidate.rs   # arlm consolidate
│   ├── persist.rs       # arlm persist
│   ├── decay.rs         # arlm decay
│   ├── mcp.rs           # MCP protocol handler
│   └── serve.rs         # arlm serve (HTTP/MCP)
└── output/
    ├── mod.rs           # Format enum
    ├── json.rs          # JsonOutput
    ├── tree.rs          # ASCII tree
    ├── markdown.rs      # Markdown
    ├── prompt.rs        # LLM prompt
    └── live_tree.rs     # Live tree para --live
```

## Comandos

| Comando | Descrição | Requer --llm |
|---------|-----------|--------------|
| `arlm index` | Indexa projeto | Não |
| `arlm search` | Busca híbrida | Não |
| `arlm context` | Contexto para agente | Não |
| `arlm query` | Consulta com RLM | Sim |
| `arlm run` | Executa RLM recursivo | Sim |
| `arlm status` | Mostra projetos indexados | Não |
| `arlm history` | Histórico de consultas | Não |
| `arlm cost` | Resumo de custos | Não |
| `arlm session` | Gerencia sessões | Não |
| `arlm consolidate` | Limpa memória | Não |
| `arlm persist` | Salva como wiki pages | Não |
| `arlm decay` | Salience decay | Não |
| `arlm serve` | HTTP/MCP server | Não |

## Flags Principais

### `arlm index`

```bash
arlm index ./meu-projeto
arlm index ./meu-projeto --ignore "dist/" --ignore "*.log"
arlm index ./meu-projeto --watch
arlm index ./meu-projeto --chunk-size 1024
```

| Flag | Descrição | Default |
|------|-----------|---------|
| `--chunk-size <N>` | Tamanho máximo por chunk | 512 |
| `--ignore <pattern>` | Padrões de ignore (múltiplos) | `.env`, `*.pem`, `*.key` |
| `--watch` / `-w` | Reindexa a cada mudança | off |

### `arlm search`

```bash
arlm search "auth middleware"
arlm search "config" --all
arlm search "error" --tier entity
arlm search "schema" --max-tokens 4000
arlm search "bug" --all --tier auto --max-tokens 8000
```

| Flag | Descrição | Default |
|------|-----------|---------|
| `--top-k <N>` | Número de resultados | 10 |
| `--all` / `-a` | Busca cross-project | off |
| `--tier <tier>` | `fts`, `entity`, `vector`, `auto` | auto |
| `--max-tokens <N>` | Limite de tokens (0=ilimitado) | 8000 |
| `--file-pattern <pat>` | Filtro por arquivo | — |
| `--min-score <f>` | Score mínimo | — |

### `arlm context`

```bash
arlm context "fix login bug"
arlm context "auth" --all --max-tokens 4000
arlm context "db schema" --tier entity
```

| Flag | Descrição | Default |
|------|-----------|---------|
| `--top-k <N>` | Número de resultados | 10 |
| `--all` / `-a` | Busca cross-project | off |
| `--tier <tier>` | `fts`, `entity`, `vector`, `auto` | auto |
| `--max-tokens <N>` | Limite de tokens (0=ilimitado) | 8000 |

### `arlm run`

```bash
arlm run "analise completa" --llm --backend openai
arlm run "refactor" --llm --backend anthropic --model claude-3.5-sonnet
arlm run "fix bug" --llm --depth 5 --max-nodes 100 --live
```

| Flag | Descrição | Default |
|------|-----------|---------|
| `--llm` | Habilita modo LLM | — |
| `--backend <name>` | openai, anthropic, ollama, gemini, deepseek, mimo | ollama |
| `--model <name>` | Modelo | — |
| `--depth <N>` | Profundidade recursão | 3 |
| `--max-nodes <N>` | Máximo de nós | 50 |
| `--concurrency <N>` | Concorrência | 4 |
| `--max-budget <USD>` | Orçamento | 1.0 |
| `--live` | Árvore em tempo real | off |

## Formatos de Saída

```bash
arlm search "query" --format json       # JSON estruturado
arlm search "query" --format tree       # Tabela colorida (default)
arlm search "query" --format markdown   # Markdown
arlm search "query" --format prompt     # Prompt para LLM
```

## Flags Globais

```
--project <path>        # Caminho do projeto (default: .)
--format <fmt>          # json|tree|markdown|prompt
--verbose, -v           # Logs detalhados
```

## Uso

```bash
# Indexar projeto
arlm index ./meu-projeto

# Buscar com verbose
arlm search "bug no login" --verbose

# Contexto formatado
arlm context "analise auth" --format prompt

# Output JSON
arlm search "error" --format json

# Com LLM
arlm run "analise completa" --llm --backend openai

# Busca cross-project
arlm search "config" --all

# Com watch mode
arlm index ./projeto --watch
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
# Debug
cargo build -p arlm-cli

# Release (otimizado)
cargo build --release -p arlm-cli

# Binary: ./target/release/arlm
```

## Testes

```bash
cargo test -p arlm-cli
```

49 testes cobrindo: todos os comandos, formatos de output, parsing, watch mode.
