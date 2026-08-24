# arlm-server

Long-running **gRPC server** para a plataforma [arlm](https://github.com/anomalyco/arlm)
(Agnostic RLM). Armazena, indexa, sumariza e executa RLM (Recursive Language Model)
para times, expondo uma API gRPC (tonic) consumível por qualquer agente de IA.

## Visão geral

O servidor gerencia **projetos (buffers)**, **sessões**, **runs de RLM**, **indexação**
(chunking + embeddings + usearch), **sumarização hierárquica** e — desde os planos
018/017 — **autenticação por refresh-token** e um **cache semântico de respostas
digeridas (QA-Cache)**, com streaming de eventos em tempo real para clientes.

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

[qa_cache]
enabled = true
novel_k = 20              # chunks digeridos numa pergunta nova (client)
provenance_k = 5          # chunks de provenance devolvidos com a resposta
sim_high = 0.90           # acima disso → reaproveita + re-digest leve
sim_floor = 0.40          # abaixo disso → trata como nova (digest completo)
max_entries_per_project = 1000
lambda_ms = 86400000      # decaimento do score LRU ponderado
cache_ttl_ms = 0          # 0 = sem TTL
```

> **Auth (plan 018):** os RPCs mutantes (`InvalidateCache`, e qualquer RPC que
> escreva estado) exigem um `Authorization: Bearer <session>` válido; operações
> de invalidação exigem role `Admin`. Clientes obtêm a sessão via `AuthRefresh`.
> O servidor é **determinístico** no QA-Cache: não invoca nenhum LLM — a
> síntese roda no client (config `arlm-llm` do usuário).

## Arquitetura

Fluxo unidirecional: `arlm-cli` → `arlm-server` (gRPC) → `arlm-core` (engine RLM) /
`arlm-storage` (SQLite + LanceDB) / `arlm-embedding` / `arlm-llm`.

- **Handlers gRPC** (`src/grpc/*`): um arquivo por grupo de RPCs.
- **`auth`** (`src/auth/mod.rs`): autenticação por refresh-token + sessões de curta
  duração (plan 018); `authenticate(md, storage)` e `require_admin(ctx)` usados
  pelos handlers que escrevem estado.
- **`store`** (`src/store/*`): camada de acesso a dados tipada e segura para o pool.
- **`summarizer`** (`src/summarizer/*`): engine de sumarização hierárquica em worker
  em background, com streaming de progresso.
- **QA-Cache (plan 017):** `AppState` carrega `question_vector_store`
  (`QuestionVectorStore`, usearch, espaço B) + `qa_config` (`QaCacheConfig`) e
  dispara um worker de eviction LRU em background; `grpc/index.rs` marca entradas
  `stale` por hash de chunk no pós-reindex.
- **`events`**: `EventHub` (broadcast) que faz a ponte engine → streams gRPC.
- **`state`**: `AppState` compartilhado (storage, llm, event hub, vector store,
  question_vector_store, qa_config, abort signals de runs).
- **`timing`**: `Timer` que emite `elapsed_ms`/`elapsed_us` estruturados via `tracing`.

## Query-Answer Cache (plan 017)

Cache semântico de respostas **digeridas no client** (o servidor não invoca LLM:
só embedding + SQLite + usearch + ops determinísticas). Fluxo:

1. Cliente → `QueryWithCache(pergunta, project)`. Servidor faz busca híbrida +
   lookup semântico no `question_vector_store` (espaço B) e decide hit/tier.
2. **HIT** → devolve `answer_text` + provenance (`source_chunk_ids`); client não
   chama LLM (0 custo). **MISS** → devolve top-K chunks crus; client faz 1 chamada
   LLM, exibe e dispara `StoreAnswer` (fire-and-forget).
3. Cada resposta recebe um `cache_id` (UUIDv7) estável → `GetAnswerById` devolve
   exatamente a mesma resposta+provenance (anti-drift para sub-agentes).
4. **Invalidação:** `InvalidateCache` com `mode=Stale` (soft, força re-digest) ou
   `Delete` (hard), mais `similarity_radius` para invalidar o cluster de perguntas
   vizinhas (cadeia de erros). Exigido role `Admin`.
5. **Staleness:** no reindex, chunks cujo hash mudou marcam as entradas de cache
   dependentes como `stale` → próxima query força re-digest com código fresco.

Configurável via `[qa_cache]` (limiares, `novel_k`, `provenance_k`, eviction).

## Testes

Os testes de integração vivem em `tests/` (fora de `src/`):

- `tests/indexing_tests.rs` — chunking, linguagem, hashing.
- `tests/store_tests.rs` — CRUD de projetos/sessões/runs.
- `tests/summarizer_tests.rs` — custo, progresso, estratégia de prompt.

## Licença

Idêntica ao workspace (MIT/Apache-2.0).
