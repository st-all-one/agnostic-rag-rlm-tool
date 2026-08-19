# arlm — Docker

Imagem Docker para o [arlm](https://github.com/user/agnostic-rlm-rs) (Agnostic RLM).

## Imagem

```
arlm/arlm:latest
```

| Propriedade | Valor |
|-------------|-------|
| Base | `debian:bookworm-slim` |
| Tamanho | ~93MB |
| Arquitetura | `linux/amd64` |
| Usuário | `arlm` (non-root) |
| Entry point | `arlm` |

## Início Rápido

### Servidor HTTP

```bash
docker run -d \
  --name arlm-server \
  -p 8080:8080 \
  -v arlm-data:/home/arlm/.arlm \
  -v ./meu-projeto:/projects/meu-projeto:ro \
  arlm/arlm:latest serve
```

### CLI interativo

```bash
docker run --rm \
  -v arlm-data:/home/arlm/.arlm \
  -v ./meu-projeto:/projects/meu-projeto:ro \
  arlm/arlm:latest context "bug no login" --project /projects/meu-projeto
```

### Docker Compose

Copie o `docker-compose.yml` da raiz do repositório:

```bash
PROJECTS_DIR=/home/user/projetos docker compose up -d
```

## Variáveis de Ambiente

| Variável | Padrão | Descrição |
|----------|--------|-----------|
| `RUST_LOG` | `info` | Nível de log (`trace`, `debug`, `info`, `warn`, `error`) |
| `ARLM_DATA_DIR` | `/home/arlm/.arlm` | Diretório de dados persistente |
| `ARLM_HOST` | `0.0.0.0` | Host para bind do servidor |
| `ARLM_PORT` | `8080` | Porta do servidor |
| `OPENAI_API_KEY` | — | Chave API OpenAI (para operações com `--llm`) |
| `ANTHROPIC_API_KEY` | — | Chave API Anthropic |
| `DEEPSEEK_API_KEY` | — | Chave API DeepSeek |
| `GEMINI_API_KEY` | — | Chave API Google Gemini |
| `PROJECTS_DIR` | `.` | Diretório de projetos (usado no compose) |

## Volumes

| Mount | Descrição | Obrigatório |
|-------|-----------|-------------|
| `/home/arlm/.arlm` | Dados persistentes (knowledge.db, vectors.lance, wiki/) | Sim |
| `/projects/*` | Projetos para indexar | Para indexação |

**IMPORTANTE:** Use **named volume** para `/home/arlm/.arlm`. Bind mounts podem causar erros de permissão (`Permission denied`) porque o container roda como usuário `arlm` (UID não-root).

### Criar named volume

```bash
docker volume create arlm-data
```

### Verificar dados

```bash
docker run --rm -v arlm-data:/data busybox ls -la /data
```

## Endpoints HTTP

Quando rodando em modo servidor (`arlm serve`):

| Método | Endpoint | Descrição |
|--------|----------|-----------|
| `GET` | `/health` | Health check (`{"status":"ok","version":"0.1.0"}`) |
| `GET` | `/status` | Lista projetos indexados |
| `GET` | `/status/:id` | Status de um run específico |
| `POST` | `/context` | Monta contexto para agente |
| `POST` | `/search` | Busca híbrida (BM25 + semantic) |
| `POST` | `/run` | RLM recursivo (requer LLM) |
| `POST` | `/index` | Indexa projeto |
| `GET` | `/events/stream/:id` | SSE streaming de eventos |

### Exemplos com curl

```bash
# Health check
curl http://localhost:8080/health

# Buscar contexto
curl -X POST http://localhost:8080/context \
  -H "Content-Type: application/json" \
  -d '{"task": "analise de bug", "project": "default"}'

# Buscar código
curl -X POST http://localhost:8080/search \
  -H "Content-Type: application/json" \
  -d '{"query": "validate_token", "top_k": 5}'
```

## Integração com Agentes

### OPencode (tools.json)

```json
{
  "tools": [{
    "name": "rlm_context",
    "command": "docker exec arlm-server arlm context \"{{task}}\" --project /projects/default"
  }]
}
```

### Claude Desktop (MCP)

```json
{
  "mcpServers": {
    "arlm": {
      "command": "docker",
      "args": ["exec", "arlm-server", "arlm", "serve", "--mcp"]
    }
  }
}
```

### HTTP API (genérico)

```bash
curl -X POST http://localhost:8080/context \
  -H "Content-Type: application/json" \
  -d '{"task": "sua tarefa"}'
```

## Segurança

- Roda como usuário **non-root** (`arlm:arlm`)
- Sem shell no container
- Projetos montados como **read-only**
- Para isolamento adicional:

```bash
docker run --read-only \
  --tmpfs /tmp:rw,noexec,nosuid \
  -v arlm-data:/home/arlm/.arlm \
  arlm/arlm:latest serve
```

## Troubleshooting

### `Permission denied` ao criar banco

```bash
# Causa: bind mount com permissões erradas
# Solução: usar named volume
docker volume create arlm-data
docker run -v arlm-data:/home/arlm/.arlm arlm/arlm:latest status
```

### Container morre imediatamente

```bash
# Verificar logs
docker logs <container-id>

# Verificar se o binário funciona
docker run --rm arlm/arlm:latest --help
```

### Servidor não responde HTTP

```bash
# Verificar se está ouvindo
docker exec <container-id> sh -c "apt-get install -y curl && curl localhost:8080/health"

# Verificar port mapping
docker port <container-id>
```

## Build Local

Se quiser construir a imagem localmente:

```bash
docker build -f docker/Dockerfile -t arlm:local .
```

Requer: Docker, ~2GB de espaço em disco durante build, ~5 min de compilação.
