# arlm — Agnostic RLM

Recursive Language Model CLI/HTTP para processamento massivo de codebases. Indexa arquivos, armazena embeddings e realiza busca híbrida (BM25 + semântica) para fornecer contexto a agentes LLM.

**Agent-agnostic:** qualquer agente (OPencode, Cursor, Aider, Pi) pode consumir sua saída.

## Arquitetura

```
arlm-cli → arlm-core → arlm-llm, arlm-search, arlm-memory
                           ↓
arlm-search → arlm-storage (SQLite + LanceDB), arlm-embedding
                           ↓
arlm-storage → rusqlite (FTS5/BM25), lancedb (HNSW vetorial)
arlm-embedding → candle (BGE-M3), memmap2, rayon
```

**7 crates** no workspace Cargo. Dados compartilhados em `~/.arlm/knowledge.db` (SQLite) + `~/.arlm/vectors.lance/` (LanceDB).

## Instalação

```bash
# Build release (recomendado)
cargo build --release

# Localizar binário
./target/release/arlm
```

### Requisitos

- Rust 1.85+ (edition 2024)
- `protoc` (protobuf-compiler) para LanceDB
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

# Servidor HTTP
arlm serve --port 8080
```

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

## HTTP API

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

## Docker

```bash
# Build
docker build -t arlm -f docker/Dockerfile .

# Executar
docker run -v arlm-data:/data arlm index /workspace
docker run -v arlm-data:/data arlm search "query" --all

# Docker Compose
docker compose up -d
```

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
