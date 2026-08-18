# Arquitetura CLI — Design Agnóstico

## Visão Geral

O `arlm` é um CLI agnóstico que funciona como ferramenta compartilhada por qualquer agente de IA. A key insight é: **o CLI não depende de nenhum agente — agentes dependem do CLI.**

```
┌──────────────────────────────────────────────────────────┐
│                    arlm CLI                               │
│                                                          │
│  arlm run "analise este codigo"                          │
│  arlm index ./projeto                                    │
│  arlm search "bug no login" --project ./x                │
│  arlm context "tarefa" --project ./x --format prompt     │
│  arlm query "pergunta" --project ./x                     │
│  arlm serve --port 8080                                  │
│                                                          │
│  Output: JSON | Tree | Markdown | Prompt                 │
└───────────────┬──────────────────────────────────────────┘
                │
    ┌───────────┼───────────┬───────────┐
    ▼           ▼           ▼           ▼
┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐
│OPencode│ │Pi Agent│ │ Cursor │ │  Aider │
│        │ │        │ │        │ │        │
│tool    │ │extension│ │command │ │ hook   │
│bash    │ │register│ │pattern │ │        │
└────────┘ └────────┘ └────────┘ └────────┘
```

## Comandos Principais

### 1. `arlm index` — Indexar Projeto

```bash
# Indexa um projeto inteiro
arlm index ./meu-projeto \
  --backend ollama \
  --model llama3 \
  --chunk-strategy code \
  --concurrency 8

# Output:
# Indexing ./meu-projeto...
# [████████████████████░░░░] 82% (1,247/1,521 chunks) 3.2s ETA: 0.7s
# Indexed 1,521 chunks in 3.9s
# Database: ~/.arlm/projects/meu-projeto/knowledge.db
# Embeddings: ~/.arlm/projects/meu-projeto/vectors.lance

# Re-indexa incrementalmente (só arquivos modificados)
arlm index ./meu-projeto --incremental

# Indexa com config customizada
arlm index ./meu-projeto \
  --chunk-size 512 \
  --chunk-overlap 64 \
  --ignore "target/,.git,node_modules"
```

### 2. `arlm search` — Busca Rápida

```bash
# Busca híbrida (BM25 + semântico + RRF)
arlm search "bug no login" --project ./meu-projeto

# Output JSON:
arlm search "bug no login" --project ./meu-projeto --format json
{
  "query": "bug no login",
  "results": [
    {
      "chunk_id": 42,
      "file": "src/auth/login.rs",
      "line_start": 120,
      "line_end": 145,
      "score": 0.89,
      "content": "fn validate_token(token: &str) -> Result<bool> {"
    }
  ],
  "duration_ms": 23
}

# Busca com filtros
arlm search "error handling" \
  --project ./meu-projeto \
  --file-pattern "*.rs" \
  --min-score 0.5 \
  --top-k 10
```

### 3. `arlm context` — Contexto para Agente

```bash
# Retorna contexto formatado como prompt para LLM
arlm context "analise o bug de autenticacao" \
  --project ./meu-projeto \
  --format prompt

# Output (prompt pronto para colar no system prompt do agente):
# ## Contexto do Projeto: meu-projeto
#
# ### Arquivos Relevantes (busca híbrida)
#
# #### src/auth/login.rs (score: 0.89)
# ```rust
# fn validate_token(token: &str) -> Result<bool> {
#     // ... código relevante ...
# }
# ```
#
# #### src/auth/middleware.rs (score: 0.76)
# ```rust
# fn check_session(req: &Request) -> Result<Session> {
#     // ... código relevante ...
# }
# ```
#
# ### Padrões Detectados
# - Tokens são validados via HMAC-SHA256
# - Sessões expiram após 30 minutos
# - Rate limiting aplicado por IP
#
# ### Histórico de Análises
# - 2024-01-15: Bug similar encontrado em auth/token.rs (resolvido)
# - 2024-01-10: Performance issue em middleware (resolvido)

# Output JSON para agentes que preferem parse:
arlm context "tarefa" --project ./x --format json

# Output markdown:
arlm context "tarefa" --project ./x --format markdown
```

### 4. `arlm run` — Executar RLM Recursivo

```bash
# Executa análise RLM completa
arlm run "analise a arquitetura deste projeto e encontre vulnerabilidades" \
  --project ./meu-projeto \
  --backend openai \
  --model gpt-4 \
  --depth 3 \
  --max-nodes 20 \
  --concurrency 4 \
  --format tree

# Output (árvore em tempo real):
# RLM run abc123 (auto, maxDepth=3)
# ├─ n1 [completed/solve] Analisar arquitetura geral ✓ (2.3s)
# ├─ n2 [running/decompose] Encontrar vulnerabilidades...
# │  ├─ n3 [completed/solve] Verificar autenticação ✓ (1.1s)
# │  ├─ n4 [completed/solve] Verificar SQL injection ✓ (1.8s)
# │  └─ n5 [running/solve] Verificar XSS... (2.1s)
# └─ n6 [pending] Sintetizar findings

# Output JSON:
arlm run "tarefa" --project ./x --format json
{
  "run_id": "abc123",
  "task": "analise a arquitetura...",
  "result": "Encontradas 3 vulnerabilidades criticas...",
  "tree": { "nodes": [...], "stats": {...} },
  "duration_ms": 12500,
  "cost_usd": 0.042
}

# Run assíncrono (background):
arlm run "tarefa" --project ./x --async
# → Retorna run_id, pode consultar com arlm status <run_id>

# Cancelar run:
arlm cancel <run_id>
```

### 5. `arlm query` — Consulta com RLM

```bash
# Consulta que usa RLM para encontrar resposta
arlm query "qual a causa raiz do bug de memoria no módulo X?" \
  --project ./meu-projeto \
  --backend openai \
  --model gpt-4

# Diferente de search (que retorna chunks brutos),
# query usa RLM para analisar recursivamente e dar uma resposta合成
```

### 6. `arlm status` / `arlm history`

```bash
# Status de runs
arlm status
# ID        STATUS    DURATION  NODES  TASK
# abc123    running   12.3s     5/20   "analise arquitetura..."
# def456    completed 8.7s      12/12  "verificar testes..."

# Histórico de consultas
arlm history --project ./meu-projeto --limit 20
# DATE              QUERY                           DURATION  RESULT_SIZE
# 2024-01-15 10:30  "bug no login"                  2.3s      1.2KB
# 2024-01-15 09:15  "arquitetura do auth module"    8.7s      4.5KB
```

### 7. `arlm consolidate` — Limpar Memória

```bash
# Consolida memória (remove duplicatas, agrega padrões)
arlm consolidate --project ./meu-projeto

# Consolida tudo
arlm consolidate --all

# Remove análises antigas
arlm consolidate --project ./meu-projeto --max-age 30d
```

### 8. `arlm serve` — HTTP API Server

```bash
# Inicia servidor HTTP para agentes remotos
arlm serve --port 8080 --host 0.0.0.0

# Endpoints:
# POST /context     → Contexto para agente
# POST /search      → Busca híbrida
# POST /run         → Executar RLM
# GET  /status/:id  → Status de run
# POST /index       → Indexar projeto
# GET  /history     → Histórico
# POST /query       → Consulta com RLM

# Agentes remotos podem usar:
curl -X POST http://localhost:8080/context \
  -H "Content-Type: application/json" \
  -d '{"task": "analise o bug", "project": "meu-projeto", "format": "prompt"}'
```

## Output Formats

### JSON (padrão para agentes)

```json
{
  "status": "ok",
  "data": { ... },
  "metadata": {
    "duration_ms": 123,
    "project": "meu-projeto",
    "version": "0.1.0"
  }
}
```

### Tree (para humanos)

```
RLM run abc123 (auto, maxDepth=3)
├─ n1 [completed/solve] Analisar arquitetura ✓ (2.3s)
├─ n2 [running/decompose] Encontrar vulnerabilidades...
│  ├─ n3 [completed/solve] Verificar autenticação ✓ (1.1s)
│  └─ n4 [running/solve] Verificar SQL injection... (1.8s)
└─ n5 [pending] Sintetizar findings
```

### Markdown (para documentação)

```markdown
## Resultado da Análise

### Vulnerabilidades Encontradas

1. **SQL Injection** (Crítico)
   - Arquivo: `src/db/query.rs:45`
   - Descrição: Query não utiliza prepared statements

2. **XSS Refletido** (Alto)
   - Arquivo: `src/views/user.rs:123`
   - Descrição: Input do usuário não é sanitizado
```

### Prompt (para colar em LLM)

```
## Contexto do Projeto: meu-projeto

### Arquivos Relevantes
[chunk 1] src/auth/login.rs (score: 0.89)
[código aqui]

### Padrões Detectados
- Autenticação via JWT
- Rate limiting por IP

### Histórico
- 2024-01-15: Bug similar resolvido
```

## Flags Globais

```bash
# Todos os comandos aceitam:
--project <path>        # Caminho do projeto
--format <fmt>          # json|tree|markdown|prompt
--verbose               # Output detalhado
--quiet                 # Output mínimo
--no-color              # Sem cores
--timeout <ms>          # Timeout global
--backend <backend>     # LLM backend (openai|anthropic|ollama|gemini)
--model <model>         # Modelo LLM
```

## Configuração

```bash
# ~/.arlm/config.toml
[defaults]
backend = "ollama"
model = "llama3"
format = "json"
concurrency = 4

[projects."meu-projeto"]
backend = "openai"
model = "gpt-4"
chunk_strategy = "code"
ignore = ["target/", ".git/"]

[schedule]
auto_index = true
interval = "1h"
max_age = "30d"
```

## Integração com Agentes

### OPencode (Tool Bash)

```json
{
  "name": "rlm_context",
  "description": "Busca contexto relevante do projeto usando RLM",
  "parameters": {
    "task": { "type": "string", "description": "Tarefa ou pergunta" }
  },
  "command": "arlm context \"{{task}}\" --project {{cwd}} --format prompt"
}
```

### Pi Agent (Extension)

```typescript
pi.registerTool({
  name: "rlm",
  execute: async (params) => {
    const result = await exec(`arlm context "${params.task}" --project ${params.cwd} --format json`);
    return JSON.parse(result);
  }
});
```

### Cursor (Command Pattern)

```json
{
  "rlm": {
    "command": "arlm context \"$ARGUMENTS\" --project . --format prompt",
    "description": "Search project context with RLM"
  }
}
```

### Aider (Hook)

```python
# .aider.conf.yml
rlm_context:
  command: "arlm context \"{question}\" --project . --format prompt"
  inject: true
```
