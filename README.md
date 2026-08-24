# arlm — Agnostic RLM

On-demand, agent-agnostic RLM para processamento de
codebases. Indexa arquivos, armazena embeddings e realiza busca híbrida (BM25 +
semântica), QA-Cache e memória sobre um **plano de dados server-first** via gRPC.
O cliente usa o **LLM local do usuário** apenas para *digest* (`query -qa`) e
*persist* — o servidor é um plano de dados puro, **sem LLM**.

**Agent-agnostic:** qualquer agente (OPencode, Cursor, Aider, Pi) pode consumir sua saída.

## Filosofia

- **On-demand, não-recursivo:** não há loop RLM recursivo nem orquestração de
  planner/solver/synthesizer. O `arlm` indexa e responde consultas sob demanda.
- **Servidor = plano de dados puro:** `arlm-server` faz indexação (chunking +
  embeddings no servidor), busca híbrida, QA-Cache, memória e histórico — tudo
  via gRPC. **Sem LLM no servidor.**
- **Cliente = cliente gRPC puro:** `arlm-cli` só usa o LLM do usuário
  (`arlm-llm`) para *digest* de QA (`query -qa`) e para *summarize* no
  `persist`. Nenhuma outra operação depende de LLM.

## Arquitetura (server-first)

```
┌──────────────────────────────────────────────────────┐
│              arlm-server  (long-running)                │
│  SQLite (FTS5/BM25) + LanceDB (vetorial) + embeddings  │
│  expõe API gRPC (tonic + prost, via arlm-proto)        │
│  sem LLM — plano de dados puro                          │
└───────────────────────────┬──────────────────────────┘
                            │ gRPC (protobuf, TLS opcional)
┌───────────────────────────┴──────────────────────────┐
│  arlm-cli  (thin gRPC client)                           │
│  init / index / search / query / memory /              │
│  persist / history / server                            │
│  usa LLM local do usuário só em query -qa / persist    │
└───────────────────────────────────────────────────────┘
```

- **9 crates**: `arlm-cli`, `arlm-core`, `arlm-storage`, `arlm-search`,
  `arlm-embedding`, `arlm-memory`, `arlm-llm`, `arlm-proto`, `arlm-server`.
- Conexão tipada por `arlm-proto` (trait `ArlmService`, RPCs sobre gRPC).
- **QA-Cache (plan 017):** o servidor faz embedding + SQLite + LanceDB e devolve
  respostas digeridas; a síntese LLM (digest) roda no **cliente** (LLM do
  usuário) com `--cache-id` para lookup determinístico 1:1 e `cache_id` estável
  (anti-drift).
- **Auth (plan 018):** refresh-tokens + sessões de curta duração com roles
  `Admin`/`NonAdmin`; RPCs mutantes exigem `Bearer` válido.
- **Sem LLM no servidor** para qualquer operação (index/search/query/memory/
  history). O LLM é usado **apenas no cliente**, para `query -qa` (digest) e
  `persist` (summarize), via `arlm-llm`.
- Manutenção (consolidate/decay) do servidor é feita por **cron + RPC admin**
  `TriggerMaintenance` (e `arlm-server admin consolidate`), não por comandos de
  CLI do usuário.

## Instalação

```bash
# Binários (server + client)
cargo build --release            # → ./target/release/arlm e ./target/release/arlm-server

# Ou via script de instalação
./install.sh                     # instala arlm e cria ~/.arlm/arlm.toml
```

### Requisitos

- Rust 1.85+ (edition 2024)
- `protoc` (protobuf-compiler) para gRPC/`arlm-proto`
- `protobuf-devel` para includes do protobuf

## Uso Rápido

```bash
# Inicializar o projeto (cria <proj>/.arlm.toml gitignored + indexa)
arlm init ./meu-projeto
arlm init ./meu-projeto --no-index     # só cria o .arlm.toml

# Indexar (o cliente faz stream do texto bruto; o servidor chunk+embed)
arlm index ./meu-projeto

# Buscar no projeto (híbrida BM25 + semântica, server-side)
arlm search "auth middleware"

# Pergunta on-demand; -qa digere via LLM local do usuário; emite cache_id
arlm query "como funciona o login?" -qa
arlm query --cache-id <id>             # lookup determinístico 1:1

# Persistir uma resposta como wiki page (usa LLM local do usuário)
arlm persist <response_id>

# Histórico de consultas do usuário (escopado por refresh token)
arlm history --limit 20

# Memória (admin): listar / obter / invalidar / manutenção
arlm memory list
arlm memory get <cache_id>
arlm memory invalidate <cache_id>
arlm memory cleanup

# Subir o servidor gRPC/MCP (plano de dados, sem /run)
arlm server
```

## Modo Servidor (gRPC)

O modelo recomendado é separar servidor e cliente:

```bash
# 1) Inicia o servidor (long-running) — dono do estado
arlm server                                        # escuta conforme server.toml
docker compose -f docker-compose.server.yml up -d   # ou via Docker

# 2) O cliente CLI conecta por gRPC
arlm --server 127.0.0.1:50051 index ./meu-projeto
arlm --server 127.0.0.1:50051 search "auth middleware"
arlm --server 127.0.0.1:50051 query "como funciona o login?" -qa
```

Sem `--server`, o CLI opera localmente sobre `~/.arlm`. O endereço do servidor
também é resolvido por `~/.arlm/arlm.toml` (`[server].addr`) ou
`ARLM_SERVER_ADDR`.

## Comandos CLI

| Comando | Descrição |
|---------|-----------|
| `arlm init [--index] [--no-index]` | Scaffold de `<proj>/.arlm.toml` (gitignored) + index |
| `arlm index <dir>` | Faz stream do texto bruto; servidor chunk+embed |
| `arlm search <query>` | Busca híbrida BM25 + semântica (server-side) |
| `arlm query <question>` | QA on-demand; `-qa` digere via LLM do usuário; `--cache-id` lookup; emite `cache_id` |
| `arlm memory list\|get\|invalidate\|cleanup` | Memória (admin, via ListMemory/GetCache/InvalidateCache/TriggerMaintenance) |
| `arlm persist <response_id>` | Escreve `wiki/<yyyymmddhhmm>_<title>.md` (summarize via LLM do usuário) |
| `arlm history [--limit] [--user]` | Histórico de consultas por usuário (escopado por refresh token) |
| `arlm server` | Hospeda o servidor gRPC/MCP (plano de dados, sem `/run`) |

**Removidos (plan 019):** `run`, `context`, `session`, `status`, `cost`,
`cancel`, `checkpoints`, `restore-page`, `wiki`, `consolidate` (CLI), `decay`
(CLI) e `entities` (CLI). A manutenção server-side (consolidate/decay) é feita
por cron + RPC admin `TriggerMaintenance` (e `arlm-server admin consolidate`).

## Flags Principais

### `arlm index`

| Flag | Descrição | Default |
|------|-----------|---------|
| `--ignore <pattern>` | Padrões de ignore (glob, múltiplos) | `.env`, `.env.*`, `*.pem`, `*.key` |

> O chunking e os embeddings ocorrem **no servidor**. O cliente apenas faz
> stream do texto bruto dos arquivos (client-streaming gRPC `IndexProject`).

### `arlm search`

| Flag | Descrição | Default |
|------|-----------|---------|
| `--top-k <N>` | Número de resultados | 10 |
| `--file-pattern <pat>` | Filtro por nome de arquivo | — |
| `--min-score <f>` | Score mínimo | — |

### `arlm query`

| Flag | Descrição | Default |
|------|-----------|---------|
| `-qa` | Digere a resposta via LLM local do usuário (emite `cache_id`) | off |
| `--cache-id <id>` | Lookup determinístico 1:1 (sem chamar LLM) | — |

## Formatos de Saída

Todos os comandos suportam 4 formatos:

```bash
arlm search "query" --format json      # JSON estruturado
arlm search "query" --format tree      # Tabela colorida (default)
arlm search "query" --format markdown  # Markdown formatado
arlm search "query" --format prompt    # Prompt para LLM
```

## Arquitetura de Dados

### Server-side (compartilhado)

O `arlm-server` é dono do estado. Por padrão (container) os dados vivem em
`/data` (configurável via `server.toml` `data_dir`):

```
/data/
├── knowledge.db          # SQLite (WAL, FTS5, metadados)
├── knowledge.db-wal      # WAL journal
└── vectors.lance/        # LanceDB (HNSW vetorial, BGE-M3)
```

Cada projeto é um `buffer` na tabela `buffers` com UUIDv7 único. Isolamento por
`buffer_id` em todas as tabelas.

### Busca Híbrida

| Camada | Componentes | Requisitos |
|--------|-------------|------------|
| BM25 | FTS5 (SQLite) | Nenhum |
| Semântica | embeddings BGE-M3 + LanceDB (HNSW) | Modelo BGE-M3 (servidor) |
| RRF | Fusão Reciprocal Rank (BM25 + semântica) | Nenhum |

> Não há mais tier `llm_rerank` no servidor: o servidor é LLM-free. O rerank
> LLM, quando aplicável, ocorre apenas no cliente (digest de `query -qa`).

## Configuração

### `server.toml` (HOST — arquivo de config do servidor)

Montado no container (ex.: `./server.toml:/etc/arlm/server.toml`). Lido de
`ARLM_SERVER_CONFIG` ou, por padrão, `/etc/arlm/server.toml`. É um **arquivo de
host** e possui **toda** a configuração do plano de dados — **não** há seção
`[llm]` (o servidor é LLM-free):

```toml
listen_addr = "0.0.0.0:50051"
data_dir = "/data"

# tls_cert = "/etc/arlm/tls/server.crt"
# tls_key  = "/etc/arlm/tls/server.key"

[embedder]
max_tokens = 512        # tamanho máximo de chunk (tokens)
overlap_tokens = 64     # sobreposição entre chunks

[qa_cache]
# parâmetros de cache semântico (anti-drift por hash de chunk)

[maintenance]
interval_secs = 3600
decay_score_floor = 0.05
```

### Config do usuário (2 escopos)

O cliente (`arlm-cli`) lê configuração do usuário em **2 escopos**, com merge
granular campo a campo (local > global):

- **Global** `~/.arlm/arlm.toml`: `[auth]` (só global: `username` +
  `refresh_token`), `[llm]` (IA do usuário), `[server] addr`.
- **Local** `.arlm.toml` (no projeto): sobrescreve campos do global + `[project]`.

`[auth]` é **somente global** e é ignorado se presente no arquivo local.
Arquivos legados `~/.arlm/config.toml` / `.arlm/config.toml` **não** são lidos.

```toml
# ~/.arlm/arlm.toml (global)
[auth]
username = "alice"
refresh_token = "..."      # obtido no login; só-global

[llm]
backend = "ollama"
model = "llama3.2"

[server]
addr = "127.0.0.1:50051"
```

```toml
# .arlm.toml (local, no projeto)
[project]
name = "meu-projeto"

[server]
addr = "10.0.0.5:50051"    # sobrescreve o global
```

## Docker (server-first)

A imagem canônica é o `arlm-server` (gRPC):

```bash
# Build da imagem do servidor
docker build -t arlm-server:latest -f Dockerfile.server .

# Subir o servidor (porta 50051, volume de dados persistido, server.toml montado)
docker compose -f docker-compose.server.yml up -d

# CLI (no host) conecta por gRPC
arlm --server 127.0.0.1:50051 index /workspace
arlm --server 127.0.0.1:50051 search "query"
```

O `docker-compose.server.yml` monta o volume `arlm-server-data` em `/data`
(configure `data_dir=/data` no `server.toml`) e monta o `server.toml` em
`/etc/arlm/server.toml`. O healthcheck usa `arlm-server status`.

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
