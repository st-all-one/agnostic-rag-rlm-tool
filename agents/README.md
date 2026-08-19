# Agent Integration — Padrões de Integração com Agentes

O `arlm` é agnóstico a agentes — qualquer agente de IA pode usá-lo via CLI ou HTTP.

## Diretórios por Agente

```
agents/
├── opencode/          # OPencode: tools.json
├── cursor/            # Cursor: commands.json
├── aider/             # Aider: .aider.conf.yml
├── claude-desktop/    # Claude Desktop: MCP config
├── pi/                # Pi Agent: TypeScript extension
└── README.md          # Este arquivo
```

## Rápido

### OPencode
```bash
cp agents/opencode/tools.json ~/.opencode/tools.json
```

### Cursor
```bash
cp agents/cursor/commands.json ~/.cursor/tools.json
```

### Aider
```bash
cp agents/aider/.aider.conf.yml .aider.conf.yml
```

### Claude Desktop
```bash
# Adicione ao ~/.claude/claude_desktop_config.json:
cp agents/claude-desktop/claude_desktop_config.json ~/.claude/
```

### Pi Agent
```bash
cp -r agents/pi/ ~/.pi/extensions/arlm/
# Adicione ao package.json do Pi:
# { "pi": { "extensions": ["~/.pi/extensions/arlm/index.ts"] } }
```

## HTTP API (Genérico)

Para qualquer agente que suporte HTTP:

```bash
# Iniciar servidor
arlm serve --port 8080

# Buscar contexto
curl -X POST http://localhost:8080/context \
  -H "Content-Type: application/json" \
  -d '{"task": "analise de bug", "project": "meu-app"}'

# Buscar código
curl -X POST http://localhost:8080/search \
  -H "Content-Type: application/json" \
  -d '{"query": "validate_token", "top_k": 5}'

# Executar RLM recursivo
curl -X POST http://localhost:8080/run \
  -H "Content-Type: application/json" \
  -d '{"task": "analise vulnerabilidades", "depth": 3}'
```

## Docker

```bash
# Servidor via Docker
docker compose up -d

# CLI via Docker
docker compose run --rm arlm-cli context "bug no login" --project /projects/meu-app
```

## Fluxo de Dados

```
Usuário → Agente → arlm CLI/HTTP → Busca Híbrida → Contexto → Agente resolve
```

Todos os agentes compartilham a mesma memória em `~/.arlm/projects/<nome>/`.
