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

## Servidor (gRPC, plano de dados)

O servidor é LLM-free e puro gRPC (não há endpoint `/run`, `/context` nem MCP
local; plan 020 removeu o modo offline do client):

```bash
# Iniciar o servidor de plano de dados
arlm-server up          # ou: docker compose -f docker-compose.server.yml up -d

# O cliente CLI conecta por gRPC (addr via .arlm.toml / ~/.arlm/arlm.toml / env)
arlm search "validate_token" --top-k 5
arlm query "como funciona o login?" -qa
```

## Docker

```bash
# Servidor via Docker
docker compose up -d

# CLI via Docker (index/search)
docker compose run --rm arlm-cli search "bug no login"
docker compose run --rm arlm-cli index /projects/meu-app
```

## Fluxo de Dados

```
Usuário → Agente → arlm CLI/HTTP → Busca Híbrida → Contexto → Agente resolve
```

Todos os agentes compartilham o mesmo projeto indexado (isolado por `buffer_id`
no servidor). A memória/histórico são server-side e escopados por usuário.
