# arags-server

Long-running **gRPC server** (plano de dados) para a plataforma
[arags](https://github.com/anomalyco/arags) (Agnostic RLM). Armazena, indexa e
responde consultas para times, expondo uma API gRPC (tonic) consumível por
qualquer agente de IA. **É um plano de dados puro e LLM-free**: não invoca
nenhum LLM (a digestão/sumarização ocorre no cliente, `arags-cli`, via o LLM do
usuário).

## Visão geral

O servidor gerencia **projetos (buffers)**, **indexação** (chunking + embeddings
no servidor + usearch HNSW), **busca híbrida**, **unified contextual query**
(plan 023: chunks + sumários RLM aprovados + mapas de exploração numa resposta,
com trust pipeline), **memória/histórico**, manutenção (consolidate/decay por
cron + RPC admin) e — desde os planos 018/017 — **autenticação por
refresh-token** e um **cache semântico de respostas digeridas (QA-Cache)**,
com operações determinísticas (sem LLM).

## Build & Run

```bash
# Desenvolvimento
cargo run -p arags-server -- up

# Release
cargo build --release -p arags-server
./target/release/arags-server up

# Checagem e testes (otimizado, 12 threads)
cargo check -p arags-server
cargo test   -p arags-server
cargo clippy -p arags-server --all-targets
```

### Subcomandos

| Comando | Descrição |
|----------|-----------|
| `up`     | (padrão) Carrega config, abre storage, sobe o servidor gRPC. |
| `status` | Consulta a saúde de um servidor em execução via `GetServerStatus` (usado pelo Docker HEALTHCHECK). |

### Docker

Imagem única do projeto: `docker/Dockerfile` (musl estático → `scratch`,
pesos MiniLM assados em `/models`, healthcheck embutido).

```bash
docker build -f docker/Dockerfile -t arags-server .
docker run -d -p 50051:50051 -v arags-data:/data arags-server
```

Detalhes e overrides em [`docker/README.md`](../../docker/README.md).

## Configuração

Arquivo de **host** montado no container (ex.: `./server.toml:/etc/arags/server.toml`),
lido de `ARAGS_SERVER_CONFIG` ou, por padrão, `/etc/arags/server.toml`. É um arquivo
de host e possui **toda** a configuração do plano de dados — **não** há seção
`[llm]` (o servidor é LLM-free). Exemplo:

```toml
listen_addr = "127.0.0.1:50051"   # env ARAGS_SERVER_ADDR sobrescreve
data_dir = "/data/arags"           # env ARAGS_DATA_DIR sobrescreve

[embedder]
model_dir = "/models"              # env ARAGS_EMBEDDER_MODEL_DIR sobrescreve

# tls_cert = "/etc/arags/tls/server.crt"   # opcional → habilita TLS
# tls_key  = "/etc/arags/tls/server.key"
# mtls_ca  = "/etc/arags/tls/ca.crt"       # exige client cert (mTLS)

pool_size = 4            # pool de escrita SQLite (1 = single-mode)
flush_interval_ms = 100  # checkpoint PASSIVE do WAL (0 = desliga)
max_batch_size = 50      # linhas por transação de indexação

[embedder]
model_dir = "/models/all-MiniLM-L6-v2"  # model.safetensors + tokenizer.json
quantization = "int8"                 # default; "none" = f32
batch_size = 64                       # chunks por request de embedding
max_tokens = 512                      # tamanho máximo de chunk (tokens)
overlap_tokens = 64                   # sobreposição entre chunks
cache = true                          # cache SQLite de embeddings

[search]
tier = "hybrid"                       # default p/ SEARCH_TIER_UNSPECIFIED
top_k = 10                            # quando o request omite max_results
max_tokens = 8000                     # budget do contexto
decay_lambda = 0.0                    # decay de saliência no serving (0 = off)
summary_ratio = 0.6                   # unified query: fatia de sumários RLM (0 = off)
summary_min_score = 0.35              # score mínimo p/ sumário entrar na fusão
exploration_enabled = true            # unified query: anexar mapas relevantes
exploration_limit = 2                 # máx. de explorações por resposta

[qa_cache]
novel_k = 20              # chunks digeridos numa pergunta nova (client)
provenance_k = 5          # chunks de provenance devolvidos com a resposta
sim_high = 0.90           # acima disso → hit de alta confiança
sim_floor = 0.40          # abaixo disso → nova pergunta (digest completo)
tier_steps = [0.90, 0.80, 0.70, 0.60, 0.50]
jaccard_min = 0.5
question_vector_dims = 1024
max_entries_per_project = 1000
eviction_lambda_ms = 604800000
eviction_interval_ms = 60000

[maintenance]
interval_secs = 3600                  # 0 = desliga o ticker
decay_score_floor = 0.05

[history]
retention_days = 90                   # purge no ticker; 0 = mantém

[exploration]
require_review = false                # plan 023: não-admins caem em pending_review
```

> Os knobs de embedding vivem **apenas** aqui — as envs `ARAGS_OLLAMA_*` (e
> demais legadas) foram substituídas pelo `[embedder]` do `server.toml`. O
> modelo é **fixo**: all-MiniLM-L6-v2 nativo em candle (sem Ollama, sem
> Python); `model_dir` aponta para o checkpoint baixado do HF.

> **Auth (plan 018):** os RPCs mutantes (`InvalidateCache`, e qualquer RPC que
> escreva estado) exigem um `Authorization: Bearer <session>` válido; operações
> de invalidação exigem role `Admin` — inclusive o review gate de explorações
> (`ReviewExploration`, plan 023) e o review de nós RLM. Clientes obtêm a
> sessão via `AuthRefresh`.
> O servidor é **LLM-free**: nenhum LLM é invocado aqui — a síntese (digest/
> summarize) roda no client (config `arags-llm` do usuário).

## Arquitetura

Fluxo: `arags-cli` → `arags-server` (gRPC, plano de dados) → `arags-storage`
(SQLite + usearch HNSW) / `arags-embedding` (chunking + embeddings) /
`arags-memory`
(memória, histórico, manutenção). Sem `arags-core` engine nem `arags-llm` no
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
  + purge de histórico (`[history] retention_days`) e RPC admin
  `TriggerMaintenance`. A consolidação/decay também **purge os vetores usearch**
  dos chunks removidos (`VectorStore` anexado ao `ConsolidationEngine`), mantendo
  o espaço vetorial em sincronia com o SQLite e evitando rebuild no bootstrap.
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
