# Integração com Agentes — Padrões Multi-Agente

## Visão Geral

O `arlm` é agnóstico a agentes — qualquer agente de IA pode usá-lo via CLI ou HTTP. Cada agente tem seu padrão de integração, mas todos compartilham a mesma memória central.

```
┌──────────────────────────────────────────────────────────────┐
│                  Memória Compartilhada                        │
│                                                              │
│  ~/.arlm/projects/meu-projeto/knowledge.db                  │
│  ~/.arlm/projects/meu-projeto/vectors.lance                 │
│                                                              │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐       │
│  │OPencode  │ │Pi Agent  │ │ Cursor   │ │  Aider   │       │
│  │          │ │          │ │          │ │          │       │
│  │ tool     │ │ extension│ │ command  │ │ hook     │       │
│  │ bash     │ │ register │ │ pattern  │ │          │       │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └────┬─────┘       │
│       │             │             │             │             │
│       └─────────────┴─────────────┴─────────────┘            │
│                         │                                    │
│                   ┌─────▼─────┐                              │
│                   │   arlm    │                              │
│                   │   CLI/    │                              │
│                   │   HTTP    │                              │
│                   └───────────┘                              │
└──────────────────────────────────────────────────────────────┘
```

## Padrão 1: OPencode (Tool Bash)

### Configuração

```json
// ~/.opencode/tools.json
{
  "tools": [
    {
      "name": "rlm_context",
      "description": "Busca contexto relevante do projeto usando RLM. Use para entender código, encontrar bugs, ou obter histórico de análises anteriores.",
      "parameters": {
        "task": {
          "type": "string",
          "description": "Tarefa, pergunta ou descrição do que precisa de contexto"
        },
        "format": {
          "type": "string",
          "enum": ["prompt", "json", "markdown"],
          "default": "prompt",
          "description": "Formato do output"
        }
      },
      "command": "arlm context \"{{task}}\" --project {{cwd}} --format {{format}}"
    },
    {
      "name": "rlm_search",
      "description": "Busca rápida no código do projeto. Retorna trechos relevantes com scores.",
      "parameters": {
        "query": {
          "type": "string",
          "description": "Termos de busca"
        },
        "top_k": {
          "type": "integer",
          "default": 5,
          "description": "Número de resultados"
        }
      },
      "command": "arlm search \"{{query}}\" --project {{cwd}} --top-k {{top_k}} --format json"
    },
    {
      "name": "rlm_run",
      "description": "Executa análise RLM recursiva. Use para tarefas complexas que precisam de decomposição.",
      "parameters": {
        "task": {
          "type": "string",
          "description": "Tarefa complexa para analisar recursivamente"
        },
        "depth": {
          "type": "integer",
          "default": 3,
          "description": "Profundidade máxima de recursão"
        }
      },
      "command": "arlm run \"{{task}}\" --project {{cwd}} --depth {{depth}} --format tree"
    }
  ]
}
```

### Uso no Agente

```
Usuário: "Analise o bug de login no meu projeto"

OPencode internamente:
1. Chama tool rlm_context com task="bug de login"
2. arlm retorna contexto formatado como prompt
3. OPencode usa o contexto para entender o problema
4. OPencode resolve o bug usando o código relevante encontrado
```

## Padrão 2: Pi Agent (Extension TypeScript)

### Configuração

```typescript
// index.ts
import { ExtensionAPI } from "@mariozechner/pi-coding-agent";
import { execSync } from "child_process";

export default function activate(pi: ExtensionAPI) {
  // Tool de contexto
  pi.registerTool({
    name: "rlm_context",
    description: "Busca contexto relevante do projeto usando RLM",
    parameters: {
      task: { type: "string", description: "Tarefa ou pergunta" },
      format: { type: "string", enum: ["prompt", "json", "markdown"], default: "prompt" },
    },
    execute: async (params) => {
      const result = execSync(
        `arlm context "${params.task}" --project ${process.cwd()} --format ${params.format}`,
        { encoding: "utf-8" }
      );
      return result;
    },
  });

  // Tool de busca
  pi.registerTool({
    name: "rlm_search",
    description: "Busca rápida no código do projeto",
    parameters: {
      query: { type: "string", description: "Termos de busca" },
      top_k: { type: "number", default: 5 },
    },
    execute: async (params) => {
      const result = execSync(
        `arlm search "${params.query}" --project ${process.cwd()} --top-k ${params.top_k} --format json`,
        { encoding: "utf-8" }
      );
      return JSON.parse(result);
    },
  });

  // Tool de run RLM
  pi.registerTool({
    name: "rlm_run",
    description: "Executa análise RLM recursiva",
    parameters: {
      task: { type: "string", description: "Tarefa para analisar" },
      depth: { type: "number", default: 3 },
    },
    execute: async (params) => {
      const result = execSync(
        `arlm run "${params.task}" --project ${process.cwd()} --depth ${params.depth} --format json`,
        { encoding: "utf-8" }
      );
      return JSON.parse(result);
    },
  });
}
```

### package.json

```json
{
  "name": "arlm-pi-extension",
  "version": "0.1.0",
  "pi": {
    "extensions": ["./index.ts"]
  },
  "dependencies": {}
}
```

## Padrão 3: Cursor (Command Pattern)

### Configuração

```json
// ~/.cursor/tools.json
{
  "commands": {
    "rlm": {
      "command": "arlm context \"$ARGUMENTS\" --project . --format prompt",
      "description": "Search project context with RLM"
    },
    "rlm-search": {
      "command": "arlm search \"$ARGUMENTS\" --project . --format json",
      "description": "Search code with RLM"
    },
    "rlm-analyze": {
      "command": "arlm run \"$ARGUMENTS\" --project . --depth 3 --format tree",
      "description": "Analyze with RLM recursively"
    }
  }
}
```

### Uso

```
/rlm bug no login
→ Cursor executa: arlm context "bug no login" --project . --format prompt
→ Output é injetado no contexto do Cursor
```

## Padrão 4: Aider (Hook)

### Configuração

```yaml
# .aider.conf.yml
rlm_context:
  command: "arlm context \"{question}\" --project . --format prompt"
  inject: true
  priority: 10

rlm_search:
  command: "arlm search \"{question}\" --project . --format json"
  inject: false
  priority: 20
```

### Hook Python

```python
# .aider/hooks/rlm_hook.py
import subprocess
import json

def on_question(question: str, **kwargs):
    # Busca contexto RLM
    result = subprocess.run(
        ["arlm", "context", question, "--project", ".", "--format", "json"],
        capture_output=True,
        text=True
    )

    if result.returncode == 0:
        data = json.loads(result.stdout)
        return {
            "context": data.get("context", ""),
            "files": data.get("files", []),
        }

    return {}
```

## Padrão 5: Claude Desktop (MCP Server)

### Configuração

```json
// ~/.claude/claude_desktop_config.json
{
  "mcpServers": {
    "arlm": {
      "command": "arlm",
      "args": ["serve", "--mcp", "--port", "8080"],
      "env": {
        "ARLM_PROJECT": "/home/user/projetos/meu-app"
      }
    }
  }
}
```

### MCP Tools

```rust
// No arlm serve --mcp
pub fn mcp_tools() -> Vec<McpTool> {
    vec![
        McpTool {
            name: "rlm_context".into(),
            description: "Busca contexto do projeto".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task": { "type": "string" }
                },
                "required": ["task"]
            }),
        },
        McpTool {
            name: "rlm_search".into(),
            description: "Busca no código".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "top_k": { "type": "integer", "default": 5 }
                },
                "required": ["query"]
            }),
        },
    ]
}
```

## Padrão 6: HTTP API (Genérico)

### Para qualquer agente que suporte HTTP:

```bash
# Servidor
arlm serve --port 8080 --host 0.0.0.0

# Endpoints
POST /context    → Contexto para agente
POST /search     → Busca híbrida
POST /run        → RLM recursivo
GET  /status/:id → Status de run
POST /index      → Indexar projeto
```

### Exemplo com curl

```bash
# Busca contexto
curl -s http://localhost:8080/context \
  -H "Content-Type: application/json" \
  -d '{"task": "bug no login", "project": "meu-app", "format": "prompt"}'

# Busca código
curl -s http://localhost:8080/search \
  -H "Content-Type: application/json" \
  -d '{"query": "validate_token", "project": "meu-app", "top_k": 5}'

# Run RLM
curl -s http://localhost:8080/run \
  -H "Content-Type: application/json" \
  -d '{"task": "analise vulnerabilidades", "project": "meu-app", "depth": 3}'
```

## Fluxo de Dados: Exemplo Completo

```
1. Usuário: "Analise o bug de login no meu projeto"

2. OPencode recebe a tarefa

3. OPencode chama tool rlm_context:
   arlm context "bug de login" --project ./meu-app --format prompt

4. arlm executa:
   a. Embedding da query (5ms)
   b. Busca semântica usearch (10ms)
   c. Busca BM25 SQLite (5ms)
   d. Fusão RRF (1ms)
   e. Recuperação dos textos (5ms)
   f. Montagem do prompt (2ms)
   Total: ~28ms

5. arlm retorna contexto formatado:
   "## Contexto do Projeto: meu-app
    ### Arquivos Relevantes
    [chunk 1] src/auth/login.rs (score: 0.89)
    fn validate_token(token: &str) -> Result<bool> { ... }
    ..."

6. OPencode usa o contexto:
   - Entende que o bug está em validate_token
   - Identifica que o token não está sendo verificado
   - Resolve o bug

7. OPencode opcionalmente salva o resultado:
   arlm save-finding "bug de login" "token não verificado" --project ./meu-app

8. Próxima vez que alguém perguntar sobre login:
   arlm context "login" --project ./meu-app
   → Inclui o histórico do bug resolvido
```

## Memória Compartilhada entre Agentes

```bash
# Agente A (OPencode) indexa o projeto
arlm index ./meu-app --backend openai

# Agente B (Pi) já pode usar o mesmo knowledge base
arlm context "bug de login" --project ./meu-app

# Agente C (Cursor) também
arlm search "validate_token" --project ./meu-app

# Todos compartilham:
# - Mesmos chunks
# - Mesmos embeddings
# - Mesmo histórico
# - Mesmos padrões extraídos
```

## Segurança

### Isolamento por projeto

```bash
# Cada projeto tem seu próprio knowledge base
~/.arlm/projects/projeto-a/  # Isolado
~/.arlm/projects/projeto-b/  # Isolado

# Agentes não podem acessar projetos que não indexaram
arlm context "tarefa" --project ./projeto-a  # OK
arlm context "tarefa" --project ./projeto-c  # Erro: não indexado
```

### Controle de acesso (futuro)

```bash
# API keys por projeto
arlm config set projeto-a.api_key sk-abc123

# Rate limiting
arlm config set projeto-a.max_requests_per_minute 60

# Audit log
arlm config set projeto-a.audit_log true
```
