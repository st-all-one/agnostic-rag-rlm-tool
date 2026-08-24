# arlm-server

Long-running **gRPC server** (plano de dados) para a plataforma
[arlm](https://github.com/anomalyco/arlm) (Agnostic RLM). Armazena, indexa e
responde consultas para times, expondo uma API gRPC (tonic) consumível por
qualquer agente de IA. **É um plano de dados puro e LLM-free**: não invoca
nenhum LLM (a digestão/sumarização ocorre no cliente, `arlm-cli`, via o LLM do
usuário).

## Visão geral

O servidor gerencia **projetos (buffers)**, **indexação** (chunking + embeddings
no servidor + LanceDB), **busca híbrida**, **memória/histórico**, manutenção
(consolidate/decay por cron + RPC admin) e — desde os planos 018/017 —
**autenticação por refresh-token** e um **cache semântico de respostas
digeridas (QA-Cache)**, com operações determinísticas (sem LLM).

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

Arquivo de **host** montado no container (ex.: `./server.toml:/etc/arlm/server.toml`),
lido de `ARLM_SERVER_CONFIG` ou, por padrão, `/etc/arlm/server.toml`. É um arquivo
de host e possui **toda** a configuração do plano de dados — **não** há seção
`[llm]` (o servidor é LLM-free). Exemplo:

```toml
listen_addr = "127.0.0.1:50051"
data_dir    = "/data"
# tls_cert / tls_key     # opcionais → habilita TLS

[embedder]
max_tokens = 512          # tamanho máximo de chunk (tokens)
overlap_tokens = 64       # sobreposição entre chunks

[qa_cache]
enabled = true
novel_k = 20              # chunks digeridos numa pergunta nova (client)
provenance_k = 5          # chunks de provenance devolvidos com a resposta
sim_high = 0.90           # acima disso → reaproveita + re-digest leve
sim_floor = 0.40          # abaixo disso → trata como nova (digest completo)
max_entries_per_project = 1000
lambda_ms = 86400000      # decaimento do score LRU ponderado
cache_ttl_ms = 0          # 0 = sem TTL

[maintenance]
interval_secs = 3600
decay_score_floor = 0.05
```

> **Auth (plan 018):** os RPCs mutantes (`InvalidateCache`, e qualquer RPC que
> escreva estado) exigem um `Authorization: Bearer <session>` válido; operações
> de invalidação exigem role `Admin`. Clientes obtêm a sessão via `AuthRefresh`.
> O servidor é **LLM-free**: nenhum LLM é invocado aqui — a síntese (digest/
> summarize) roda no client (config `arlm-llm` do usuário).

## Arquitetura

Fluxo: `arlm-cli` → `arlm-server` (gRPC, plano de dados) → `arlm-storage`
(SQLite + LanceDB) / `arlm-embedding` (chunking + embeddings) / `arlm-memory`
(memória, histórico, manutenção). Sem `arlm-core` engine nem `arlm-llm` no
servidor.

- **Handlers gRPC** (`src/grpc/*`): um arquivo por grupo de RPCs
  (`index`, `search`, `query_cache`, `memory`, `history`, `status`, `admin`).
- **`auth`** (`src/auth/mod.rs`): autenticação por refresh-token + sessões de curta
  duração (plan 018); `authenticate(md, storage)` e `require_admin(ctx)` usados
  pelos handlers que escrevem estado.
- **`store`** (`src/store/*`): camada de acesso a dados tipada e segura para o pool.
- **QA-Cache (plan 017):** `AppState` carrega `question_vector_store`
  (`QuestionVectorStore`, espaço B) + `qa_config` (`QaCacheConfig`) e
  dispara um worker de eviction LRU em background; `grpc/index.rs` marca entradas
  `stale` por hash de chunk no pós-reindex.
- **`maintenance`** (`src/maintenance.rs`): consolidação/decay agendados (cron)
  e RPC admin `TriggerMaintenance`.
- **`state`**: `AppState` compartilhado (storage, embedder, vector store,
  question_vector_store, qa_config, maintenance config).
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
- `tests/store_tests.rs` — CRUD de projetos/memória/histórico.

## Licença

Idêntica ao workspace (MIT/Apache-2.0).
