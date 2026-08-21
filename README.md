# arlm — Agnostic RLM

Recursive Language Model CLI/HTTP para processamento massivo de codebases. Indexa arquivos, armazena embeddings e realiza busca híbrida (BM25 + semântica) para fornecer contexto a agentes LLM.

**Agent-agnostic:** qualquer agente (OPencode, Cursor, Aider, Pi) pode consumir sua saída.

## Arquitetura (server-first)

O `arlm` é **server-first** (ver `plan/016-server-first-architecture.md`): o
`arlm-server` é o processo primário e de longa duração, e o `arlm-cli` é um
cliente gRPC fino que se comunica com ele.

```
┌──────────────────────────────────────────────────────┐
│                  arlm-server  (long-running)           │
│  SQLite (pool r2d2) + LanceDB (vetorial) + summarizer  │
│  expõe API gRPC (tonic + prost, via arlm-proto)       │
└───────────────────────────┬──────────────────────────┘
                              │ gRPC (protobuf, TLS opcional)
┌───────────────────────────┴──────────────────────────┐
│  arlm-cli  (thin gRPC client)  ── index/search/run…   │
└───────────────────────────────────────────────────────┘
```

- **9 crates**: `arlm-cli`, `arlm-core`, `arlm-storage`, `arlm-search`,
  `arlm-embedding`, `arlm-memory`, `arlm-llm`, `arlm-proto`, `arlm-server`.
- Conexão tipada por `arlm-proto` (trait `ArlmService`, 19 RPCs).
- **Sem LLM para index/search/context/query** (embeddings usam
  *fallback hash* determinístico e offline por padrão; BGE-M3 via `candle`
  é opcional). LLM é *opt-in* (`--llm`) apenas para `run`/summarize.
- O CLI também roda **localmente** (sem `--server`), operando diretamente
  sobre `~/.arlm` (retrocompatibilidade).

## Instalação

```bash
# Binários (server + client)
cargo build --release            # → ./target/release/arlm e ./target/release/arlm-server

# Ou via script de instalação
./install.sh                     # instala arlm e cria ~/.arlm/config.toml
```

### Requisitos

- Rust 1.85+ (edition 2024)
- `protoc` (protobuf-compiler) para gRPC/`arlm-proto`
- `protobuf-devel` para includes do protobuf

## Uso Rápido

```bash
# Indexar um projeto
arlm index ./meu-projeto

# Indexar com ignore patterns
arlm index ./meu-projeto --ignore "dist/" --ignore "*.log"

# Indexar com watch mode (reindexa automaticamente)
arlm index ./meu-projeto --watch

# Buscar no projeto
arlm search "auth middleware"

# Buscar em todos os projetos indexados
arlm search "config handling" --all

# Buscar com tier específico
arlm search "error handling" --tier entity

# Buscar com limite de tokens
arlm search "database schema" --max-tokens 4000

# Construir contexto para um agente
arlm context "fix login bug" --all --tier auto --max-tokens 8000

# Ver status dos projetos
arlm status

# Servidor HTTP (legacy/local, opcional)
arlm serve --port 8080
```

## Modo Servidor (gRPC)

O modelo recomendado é separar servidor e cliente:

```bash
# 1) Inicia o servidor (long-running) — dono do estado
arlm-server up                                   # escuta 127.0.0.1:50051
docker compose -f docker-compose.server.yml up -d   # ou via Docker

# 2) O cliente CLI conecta por gRPC
arlm --server 127.0.0.1:50051 index ./meu-projeto
arlm --server 127.0.0.1:50051 search "auth middleware"
arlm --server 127.0.0.1:50051 context "fix login bug"
arlm --server 127.0.0.1:50051 status
```

Sem `--server`, o CLI opera localmente sobre `~/.arlm`. O endereço do servidor
também é resolvido por `~/.arlm/config.toml` (`[server].addr`) ou
`ARLM_SERVER_ADDR`.

## Comandos CLI

| Comando | Descrição |
|---------|-----------|
| `arlm index <dir>` | Indexa um diretório (chunking + metadados) |
| `arlm search <query>` | Busca híbrida BM25 + semântica |
| `arlm context <task>` | Monta contexto para agente LLM |
| `arlm run <task>` | Executa RLM recursivo (requer `--llm`) |
| `arlm query <question>` | Pergunta com análise LLM |
| `arlm status` | Lista projetos indexados |
| `arlm history` | Histórico de consultas |
| `arlm session create/list` | Sessões multi-turn |
| `arlm consolidate` | Dedup e cleanup de memória |
| `arlm persist` | Salva resultados como wiki pages |
| `arlm decay` | Salience decay em chunks antigos |
| `arlm serve` | Servidor HTTP/MCP |

## Flags Principais

### `arlm index`

| Flag | Descrição | Default |
|------|-----------|---------|
| `--chunk-size <N>` | Tamanho máximo por chunk (tokens) | 512 |
| `--ignore <pattern>` | Padrões de ignore (glob, múltiplos) | `.env`, `.env.*`, `*.pem`, `*.key` |
| `--watch` / `-w` | Reindexa automaticamente a cada mudança | off |

### `arlm search`

| Flag | Descrição | Default |
|------|-----------|---------|
| `--top-k <N>` | Número de resultados | 10 |
| `--all` / `-a` | Busca em todos os projetos indexados | off |
| `--tier <tier>` | `fts`, `entity`, `vector`, `auto` | auto |
| `--max-tokens <N>` | Limite de tokens na saída (0=ilimitado) | 8000 |
| `--file-pattern <pat>` | Filtro por nome de arquivo | — |
| `--min-score <f>` | Score mínimo | — |

### `arlm context`

| Flag | Descrição | Default |
|------|-----------|---------|
| `--top-k <N>` | Número de resultados | 10 |
| `--all` / `-a` | Busca em todos os projetos | off |
| `--tier <tier>` | `fts`, `entity`, `vector`, `auto` | auto |
| `--max-tokens <N>` | Limite de tokens na saída (0=ilimitado) | 8000 |

### `arlm run`

| Flag | Descrição | Default |
|------|-----------|---------|
| `--llm` | Habilita modo LLM (obrigatório) | — |
| `--backend <name>` | Backend: openai, anthropic, ollama, gemini, deepseek, mimo | ollama |
| `--model <name>` | Modelo a usar | — |
| `--depth <N>` | Profundidade máxima de recursão | 3 |
| `--max-nodes <N>` | Número máximo de nós | 50 |
| `--concurrency <N>` | Limite de concorrência | 4 |
| `--max-budget <USD>` | Orçamento máximo em USD | 1.0 |
| `--live` | Renderiza árvore RLM em tempo real | off |

## Formatos de Saída

Todos os comandos suportam 4 formatos:

```bash
arlm search "query" --format json      # JSON estruturado
arlm search "query" --format tree      # Tabela colorida (default)
arlm search "query" --format markdown  # Markdown formatado
arlm search "query" --format prompt    # Prompt para LLM
```

## Arquitetura de Dados

### Single Database

Todos os projetos compartilham `~/.arlm/knowledge.db`:

```
~/.arlm/
├── knowledge.db          # SQLite (WAL, FTS5, metadados)
├── knowledge.db-wal      # WAL journal
└── vectors.lance/        # LanceDB (HNSW vetorial, 1024-dim BGE-M3)
```

Cada projeto é um `buffer` na tabela `buffers` com UUIDv7 único. Isolamento por `buffer_id` em todas as tabelas.

### Busca Híbrida (Tiers)

| Tier | Componentes | Requisitos |
|------|-------------|------------|
| `fts` | BM25 (FTS5) | Nenhum |
| `entity` | BM25 + regex entities | Nenhum |
| `vector` | BM25 + entity + embeddings | Modelo BGE-M3 |
| `llm_rerank` | Tier 2 + LLM reranker | Backend LLM |

### Token Budget

`--max-tokens` controla o tamanho da saída. chunks são mantidos/truncados por score decrescente até caber no budget:

```bash
# Contexto enxuto (4k tokens)
arlm context "auth" --max-tokens 4000

# Contexto completo (ilimitado)
arlm context "auth" --max-tokens 0
```

## HTTP API (legacy/local)

> O servidor canônico é o `arlm-server` sobre **gRPC** (ver Modo Servidor acima).
> `arlm serve` é um servidor HTTP/MCP **local/opcional** mantido para
> retrocompatibilidade e integração MCP.

```bash
arlm serve --port 8080
```

| Endpoint | Método | Descrição |
|----------|--------|-----------|
| `GET /health` | GET | Health check |
| `GET /metrics` | GET | Métricas Prometheus |
| `POST /search` | POST | Busca no projeto |
| `POST /context` | POST | Monta contexto |
| `POST /run` | POST | Executa RLM recursivo |
| `POST /index` | POST | Indexa diretório |
| `GET /status` | GET | Lista projetos |
| `POST /mcp` | POST | Endpoint MCP (opcional) |

## MCP (Model Context Protocol)

```bash
arlm serve --mcp  # Habilita endpoint /mcp
```

Ferramentas disponíveis:
- `rlm_context` — Busca contexto para uma tarefa
- `rlm_search` — Busca código com BM25 híbrido

## Docker (server-first)

A imagem canônica é o `arlm-server` (gRPC):

```bash
# Build da imagem do servidor
docker build -t arlm-server:latest -f Dockerfile.server .

# Subir o servidor (porta 50051, volume de dados persistido)
docker compose -f docker-compose.server.yml up -d

# CLI (no host) conecta por gRPC
arlm --server 127.0.0.1:50051 index /workspace
arlm --server 127.0.0.1:50051 search "query"
```

O `docker-compose.server.yml` monta o volume `arlm-server-data` em `/data`
(configure `ARLM_DATA_DIR=/data`) e expõe `50051` (bind `0.0.0.0` via
`ARLM_SERVER_ADDR`). O healthcheck usa `arlm-server status`.

> **Indexação em Docker (client-streaming):** o servidor **não** lê o filesystem
> do cliente. A CLI descobre e lê os arquivos localmente e faz *stream* dos bytes
> para o servidor via gRPC (`IndexProject` é client-streaming). Portanto **não é
> necessário montar o projeto no container** — basta apontar a CLI para o caminho
> local:
>
> ```bash
> arlm --server 127.0.0.1:50051 index /caminho/do/projeto
> ```
>
> Por padrão, caminhos sensíveis/ignorados (`.env`, `.vscode`, `.github`,
> `.gitlab`, `.zed`, vendors, …) **não** são enviados. Use `--force-include=`
> para enviá-los explicitamente.

## Desenvolvimento

```bash
# Build dev
cargo build

# Rodar testes
cargo test --workspace

# Lint e format
cargo clippy --workspace -- -D warnings
cargo fmt -- --check

# Benchmarks
cargo bench
```

## Configuração de Build

`.cargo/config.toml` (incluído no repositório):

```toml
[build]
jobs = 8
rustflags = ["-C", "target-cpu=native"]

[env]
PROTOC = "/usr/bin/protoc"
```

## Licença

MIT OR Apache-2.0
