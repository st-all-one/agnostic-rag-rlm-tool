# Arquitetura CLI — Design Agnóstico

## Visão Geral

O `arags` é um CLI agnóstico que funciona como ferramenta compartilhada por qualquer agente de IA. A key insight é: **o CLI não depende de nenhum agente — agentes dependem do CLI.**

```
┌──────────────────────────────────────────────────────────┐
│                    arags CLI                               │
│                                                          │
│  arags run "analise este codigo"                          │
│  arags index ./projeto                                    │
│  arags search "bug no login" --project ./x                │
│  arags context "tarefa" --project ./x --format prompt     │
│  arags query "pergunta" --project ./x                     │
│  arags serve --port 8080                                  │
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

### 1. `arags index` — Indexar Projeto

```bash
# Indexa um projeto inteiro
arags index ./meu-projeto \
  --backend ollama \
  --model llama3 \
  --chunk-strategy code \
  --concurrency 8

# Output:
# Indexing ./meu-projeto...
# [████████████████████░░░░] 82% (1,247/1,521 chunks) 3.2s ETA: 0.7s
# Indexed 1,521 chunks in 3.9s
# Database: ~/.arags/projects/meu-projeto/knowledge.db
# Embeddings: ~/.arags/projects/meu-projeto/vectors.lance

# Re-indexa incrementalmente (só arquivos modificados)
arags index ./meu-projeto --incremental

# Indexa com config customizada
arags index ./meu-projeto \
  --chunk-size 512 \
  --chunk-overlap 64 \
  --ignore "target/,.git,node_modules"
```

### 2. `arags search` — Busca Rápida

```bash
# Busca padrão (determinística, sem LLM)
arags search "bug no login" --project ./meu-projeto
# → usa tier entity (BM25 + entity RRF), ~8ms

# Busca FTS5 puro (mais rápida)
arags search "validate_token" --project ./meu-projeto --tier fts
# → usa apenas BM25, ~5ms

# Busca com embeddings (requer embeddings pré-computados)
arags search "bug de autenticação" --project ./meu-projeto --tier vector
# → BM25 + entity + vector RRF, ~21ms

# Busca com LLM rerank (requer --llm)
arags search "bug complexo" --project ./meu-projeto --llm
# → Tier 2 + LLM rerank, ~200ms

# Output JSON:
arags search "bug no login" --project ./meu-projeto --format json
{
  "query": "bug no login",
  "tier": "entity",
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
  "duration_ms": 8
}

# Busca com filtros
arags search "error handling" \
  --project ./meu-projeto \
  --file-pattern "*.rs" \
  --min-score 0.5 \
  --top-k 10

# Busca + persist (salva resultado como markdown)
arags search "bug no login" --project ./meu-projeto --persist
# → salva em .arags/wiki/searches/2024-01-15_bug-no-login.md
```

### 3. `arags context` — Contexto para Agente

```bash
# Retorna contexto formatado como prompt para LLM
arags context "analise o bug de autenticacao" \
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
arags context "tarefa" --project ./x --format json

# Output markdown:
arags context "tarefa" --project ./x --format markdown
```

### 4. `arags run` — Executar RLM Recursivo (REQUER --llm)

```bash
# ⚠️ arags run REQUER --llm (modo determinístico não suporta recursão)
arags run "analise a arquitetura deste projeto e encontre vulnerabilidades" \
  --project ./meu-projeto \
  --llm \
  --backend openai \
  --model gpt-4 \
  --depth 3 \
  --max-nodes 20 \
  --concurrency 4 \
  --format tree

# Sem --llm, arags run retorna erro:
$ arags run "analise..." --project ./x
error: `arags run` requires --llm flag. Use `arags search` or `arags context` for deterministic operations.

# Com --llm --persist, salva a análise como markdown:
arags run "analise completa" --project ./x --llm --persist
# → salva em .arags/wiki/analyses/001-analise-completa.md

# Output (árvore em tempo real):
# RLM run abc123 (auto, maxDepth=3)
# ├─ n1 [completed/solve] Analisar arquitetura geral ✓ (2.3s)
# ├─ n2 [running/decompose] Encontrar vulnerabilidades...
# │  ├─ n3 [completed/solve] Verificar autenticação ✓ (1.1s)
# │  ├─ n4 [completed/solve] Verificar SQL injection ✓ (1.8s)
# │  └─ n5 [running/solve] Verificar XSS... (2.1s)
# └─ n6 [pending] Sintetizar findings

# Output JSON:
arags run "tarefa" --project ./x --llm --format json
{
  "run_id": "abc123",
  "task": "analise a arquitetura...",
  "result": "Encontradas 3 vulnerabilidades criticas...",
  "tree": { "nodes": [...], "stats": {...} },
  "duration_ms": 12500,
  "cost_usd": 0.042
}

# Run assíncrono (background):
arags run "tarefa" --project ./x --llm --async
# → Retorna run_id, pode consultar com arags status <run_id>

# Cancelar run:
arags cancel <run_id>
```

### 5. `arags query` — Consulta com RLM

```bash
# Consulta que usa RLM para encontrar resposta
arags query "qual a causa raiz do bug de memoria no módulo X?" \
  --project ./meu-projeto \
  --backend openai \
  --model gpt-4

# Diferente de search (que retorna chunks brutos),
# query usa RLM para analisar recursivamente e dar uma resposta合成
```

### 6. `arags status` / `arags history`

```bash
# Status de runs
arags status
# ID        STATUS    DURATION  NODES  TASK
# abc123    running   12.3s     5/20   "analise arquitetura..."
# def456    completed 8.7s      12/12  "verificar testes..."

# Histórico de consultas
arags history --project ./meu-projeto --limit 20
# DATE              QUERY                           DURATION  RESULT_SIZE
# 2024-01-15 10:30  "bug no login"                  2.3s      1.2KB
# 2024-01-15 09:15  "arquitetura do auth module"    8.7s      4.5KB
```

### 7. `arags consolidate` — Limpar Memória

```bash
# Consolida memória (determinístico: merge por hash + dedup)
arags consolidate --project ./meu-projeto

# Consolida com LLM (páginas coerentes, extrai decisions/gotchas)
arags consolidate --project ./meu-projeto --llm

# Consolida tudo
arags consolidate --all

# Remove análises antigas
arags consolidate --project ./meu-projeto --max-age 30d
```

### 8. `arags persist` — Salvar Conhecimento como Markdown

```bash
# Persiste busca recente
arags search "bug login" --persist
# → salva em .arags/wiki/searches/2024-01-15_bug-login.md

# Persiste contexto formatado
arags context "analise auth" --persist
# → salva em .arags/wiki/analyses/001-auth-analysis.md

# Persiste nota manual
arags persist --path "decisions/0007-db.md" \
  --body "# Decidimos usar Postgres\n\nMotivo: ..."
# → salva em .arags/wiki/decisions/0007-db.md

# Persiste com tag
arags persist --path "gotchas/001-unwraps.md" \
  --body "# Não usar unwrap em produção" \
  --pinned
# → pinned: true, sobrevive ao decay

# Persiste sessão
arags session persist s_abc123
# → salva em .arags/wiki/sessions/s_abc123.md
```

### 9. `arags decay` — Retenção e Esquecimento

```bash
# Roda decay (dry run — mostra o que seria removido)
arags decay --project ./x --dry-run

# Roda decay (aplica)
arags decay --project ./x

# Decay global (todos os projetos)
arags decay --all

# Hard delete de tombstones antigas
arags decay --purge --older-than 180d

# Mantém pinned e rules (sempre sobrevivem)
# evicted pages ficam como tombstone por hard_delete_days
```

### 10. `arags checkpoints` / `arags restore-page`

```bash
# Lista commits recentes da wiki
arags checkpoints --project ./x

# Restaura uma página de um commit anterior
arags restore-page --path "decisions/001-db.md" --from abc123
```

### 11. `arags entities` — Entidades do Projeto

```bash
# Lista entidades extraídas
arags entities --project ./x

# Top 50 entidades
arags entities --project ./x --top 50

# Busca por entidade
arags entities --project ./x --search "jwt"
```

### 8. `arags serve` — HTTP API Server

```bash
# Inicia servidor HTTP para agentes remotos
arags serve --port 8080 --host 0.0.0.0

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

# Modo determinístico (padrão — sem LLM):
--tier <tier>           # fts|entity|vector (padrão: entity)
--persist               # Salva output como markdown no projeto
--persist-path <path>   # Path customizado dentro de .arags/wiki/

# Modo LLM (opt-in — requer --llm):
--llm                   # Ativa LLM para esta operação
--backend <backend>     # LLM backend (openai|anthropic|ollama|gemini)
--model <model>         # Modelo LLM
```

## Configuração

```bash
# ~/.arags/config.toml
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
  "command": "arags context \"{{task}}\" --project {{cwd}} --format prompt"
}
```

### Pi Agent (Extension)

```typescript
pi.registerTool({
  name: "rlm",
  execute: async (params) => {
    const result = await exec(`arags context "${params.task}" --project ${params.cwd} --format json`);
    return JSON.parse(result);
  }
});
```

### Cursor (Command Pattern)

```json
{
  "rlm": {
    "command": "arags context \"$ARGUMENTS\" --project . --format prompt",
    "description": "Search project context with RLM"
  }
}
```

### Aider (Hook)

```python
# .aider.conf.yml
rlm_context:
  command: "arags context \"{question}\" --project . --format prompt"
  inject: true
```
