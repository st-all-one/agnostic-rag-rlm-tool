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

## Servidor (gRPC/MCP, plano de dados)

Para qualquer agente que suporte gRPC/MCP (o servidor é LLM-free; não há
endpoint `/run` nem `/context`):

```bash
# Iniciar o servidor de plano de dados (gRPC + MCP)
arlm server

# O cliente CLI conecta por gRPC
arlm --server 127.0.0.1:50051 search "validate_token" --top-k 5
arlm --server 127.0.0.1:50051 query "como funciona o login?" -qa
```

## Docker

```bash
# Servidor via Docker
docker compose up -d

# CLI via Docker (index/search; context e run foram removidos no plan 019)
docker compose run --rm arlm-cli search "bug no login"
docker compose run --rm arlm-cli index /projects/meu-app
```

## Fluxo de Dados

```
Usuário → Agente → arlm CLI/HTTP → Busca Híbrida → Contexto → Agente resolve
```

Todos os agentes compartilham o mesmo projeto indexado (isolado por `buffer_id`
no servidor). A memória/histórico são server-side e escopados por usuário.
