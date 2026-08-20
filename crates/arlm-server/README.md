# arlm-server

Long-running **gRPC server** para a plataforma [arlm](https://github.com/anomalyco/arlm)
(Agnostic RLM). Armazena, indexa, sumariza e executa RLM (Recursive Language Model)
para times, expondo uma API gRPC (tonic) consumível por qualquer agente de IA.

## Visão geral

O servidor gerencia **projetos (buffers)**, **sessões**, **runs de RLM**, **indexação**
(chunking + embeddings + LanceDB) e **sumarização hierárquica**, com streaming de
eventos em tempo real para clientes.

## Build & Run

```bash
# Desenvolvimento
cargo run -p arlm-server -- up

# Release
cargo build --release -p arlm-server
./target/release/arlm-server up

# Checagem e testes (otimizado, 12 threads)
cargo check -p arlm-server
cargo test   -p arlm-server
cargo clippy -p arlm-server --all-targets
```

### Subcomandos

| Comando | Descrição |
|----------|-----------|
| `up`     | (padrão) Carrega config, abre storage, sobe o servidor gRPC. |
| `status` | Consulta a saúde de um servidor em execução via `GetServerStatus` (usado pelo Docker HEALTHCHECK). |

### Docker

```bash
# Build + run via compose (porta 50051, comando `up`)
docker compose -f docker-compose.server.yml up --build
```

O `Dockerfile.server` expõe a porta `50051` e roda `HEALTHCHECK CMD arlm-server status`.

## Configuração

Carregada de `.arlm/config.toml` (local → global) ou env `ARLM_SERVER_ADDR`.
Exemplo:

```toml
listen_addr = "127.0.0.1:50051"
data_dir    = "/var/lib/arlm"
pool_size   = 4

[llm]
backend = "ollama"        # openai | anthropic | ollama | gemini | deepseek | mimo
model   = "qwen2.5-coder:7b"
# api_key = "..."         # opcional; cai no env da backend se ausente
# base_url = "..."        # opcional

# tls_cert / tls_key     # opcionais → habilita TLS
```

## Arquitetura

Fluxo unidirecional: `arlm-cli` → `arlm-server` (gRPC) → `arlm-core` (engine RLM) /
`arlm-storage` (SQLite + LanceDB) / `arlm-embedding` / `arlm-llm`.

- **Handlers gRPC** (`src/grpc/*`): um arquivo por grupo de RPCs.
- **`store`** (`src/store/*`): camada de acesso a dados tipada e segura para o pool.
- **`summarizer`** (`src/summarizer/*`): engine de sumarização hierárquica em worker
  em background, com streaming de progresso.
- **`events`**: `EventHub` (broadcast) que faz a ponte engine → streams gRPC.
- **`state`**: `AppState` compartilhado (storage, llm, event hub, vector store,
  abort signals de runs).
- **`timing`**: `Timer` que emite `elapsed_ms`/`elapsed_us` estruturados via `tracing`.

## Testes

Os testes de integração vivem em `tests/` (fora de `src/`):

- `tests/indexing_tests.rs` — chunking, linguagem, hashing.
- `tests/store_tests.rs` — CRUD de projetos/sessões/runs.
- `tests/summarizer_tests.rs` — custo, progresso, estratégia de prompt.

## Licença

Idêntica ao workspace (MIT/Apache-2.0).
