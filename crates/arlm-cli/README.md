# arlm-cli

Interface de linha de comando para o arlm — agent-agnostic.

## Responsabilidades

- **CLI**: Parsing de argumentos com clap derive
- **Comandos**: 10 subcomandos para todas as operações
- **Output**: 4 formatos (JSON, Tree, Markdown, Prompt)
- **Allocator**: mimalloc para performance
- **Logging**: Flags --verbose para logs estruturados

## Estrutura

```
src/
├── main.rs              # Entry point, mimalloc, clap
├── util.rs              # project_dirs() helper
├── commands/
│   ├── mod.rs
│   ├── run.rs           # arlm run "tarefa" (--llm)
│   ├── index.rs         # arlm index ./projeto
│   ├── search.rs        # arlm search "query"
│   ├── query.rs         # arlm query "pergunta"
│   ├── context.rs       # arlm context "tarefa"
│   ├── status.rs        # arlm status
│   ├── history.rs       # arlm history
│   ├── cost.rs          # arlm cost
│   ├── session.rs       # arlm session create/resume
│   ├── consolidate.rs   # arlm consolidate
│   └── serve.rs         # arlm serve (HTTP)
└── output/
    ├── mod.rs           # Format enum
    ├── json.rs          # JsonOutput
    ├── tree.rs          # ASCII tree
    ├── markdown.rs      # Markdown
    └── prompt.rs        # LLM prompt
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
| `arlm serve` | HTTP server | Não |

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
```

## Flags Globais

```
--project <path>        # Caminho do projeto
--format <fmt>          # json|tree|markdown|prompt
--verbose, -v           # Logs detalhados
--quiet                 # Output mínimo
--no-color              # Sem cores
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

20 testes cobrindo: todos os comandos, formatos de output, parsing.
