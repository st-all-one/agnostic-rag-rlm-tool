# 1. Panorama Geral da Arquitetura

> Server-first: o **estado e todo o processamento de dados vivem no
> `arags-server`**; o `arags-cli` é um cliente gRPC fino; o LLM é sempre **o do
> usuário**, nunca do servidor.

## 1.1 Princípios

1. **On-demand, não-recursivo** — não há loop de agente nem orquestração
   planner/solver/synthesizer no servidor. Indexa, busca, responde.
2. **Servidor = plano de dados puro (LLM-free)** — indexação (chunking +
   embeddings), busca híbrida, QA-Cache, memória, histórico, manutenção. Nenhum
   crate de LLM no grafo de dependências do servidor.
3. **Cliente = gRPC puro** — só usa LLM local (`arags-llm`) em dois pontos:
   *digest* da resposta (`query -qa`) e *summarize* (`persist`). Não há modo
   offline; quem quiser "offline" sobe o próprio `arags-server`.
4. **Agent-agnostic** — qualquer agente (OPencode, Cursor, Aider, Pi,
   Claude…) consome a saída via CLI/gRPC; formatos de saída pensados para
   máquina (`text`, `jsonl`, `full_json`).
5. **Confiança explícita** — todo conhecimento derivado (QA cacheada, sumário
   RLM, mapa de exploração) carrega proveniência por hash, staleness automático
   e gates de review.

## 1.2 Diagrama macro

```
┌───────────────────────────────────────────────────────────────┐
│                 arags-server  (long-running)                  │
│                                                               │
│  gRPC API (tonic + prost, arags-proto)                        │
│    ├── index:   IndexProject (client-streaming)               │
│    ├── search:  Search / BuildContext (unified query)         │
│    ├── qa:      QueryWithCache / StoreAnswer / GetAnswerById  │
│    ├── memory:  ListMemory / GetCache / InvalidateCache       │
│    ├── history: GetHistory                                    │
│    ├── rlm:     ClaimRlmJob / CompleteRlmJob / ReviewRlmNode…│
│    ├── explor.: Persist/Search/Get/Feedback/ReviewExploration │
│    └── admin:   AuthRefresh / TriggerMaintenance / Status     │
│                                                               │
│  arags-storage   SQLite (WAL + FTS5/BM25)                     │
│                  + usearch HNSW × 4 espaços vetoriais         │
│  arags-embedding chunking + all-MiniLM-L6-v2 (candle, INT8)   │
│  arags-search    BM25 ⊕ semântica → RRF → budget/unify        │
│  arags-memory    memória, histórico, manutenção               │
│  SEM LLM em nenhum ponto                                      │
└──────────────────────────┬────────────────────────────────────┘
                           │ gRPC (protobuf; TLS/mTLS opcional)
┌──────────────────────────┴────────────────────────────────────┐
│                    arags-cli  (thin client)                   │
│  init · index · watch-daemon · search · query · memory ·      │
│  persist · history · explore · volunteer                      │
│  retry/backoff · Bearer interceptor (AuthRefresh) · mTLS      │
│  LLM do usuário APENAS em: query -qa (digest), persist        │
│  (summarize), volunteer (síntese RLM)                         │
└───────────────────────────────────────────────────────────────┘
```

## 1.3 Os 9 crates

| Crate | Papel | Depende de |
|-------|-------|------------|
| `arags-cli` | Binário `arags`: parsing clap, dispatch, output multi-formato, watch daemon, volunteer, QA digest | core, proto, llm |
| `arags-server` | Binário `arags-server`: handlers gRPC, auth, config host, indexação server-side, manutenção | storage, embedding, search, memory, core, proto |
| `arags-core` | Tipos compartilhados (`EMBEDDING_DIMS=384`, RLM payload/prioridades, score de confiança de exploração) | — |
| `arags-proto` | Contrato tipado gRPC/protobuf (`service.proto` + domínios); trait `AragsService` | prost/tonic |
| `arags-storage` | SQLite (metadados, FTS5, tokens, jobs RLM) + usearch (HNSW, persistência debounced) | rusqlite, usearch |
| `arags-embedding` | Chunking por estratégia (code/text/markdown) + embedder nativo MiniLM INT8 | candle |
| `arags-search` | Busca híbrida: FTS5 BM25 + vetorial + fusão RRF (tie-break determinístico) | storage, core |
| `arags-memory` | Memória multi-projeto, knowledge base, histórico, consolidate/decay | storage |
| `arags-llm` | Abstração de backends LLM **client-side**: famílias openai/anthropic/gemini/ollama, retry, fallback, pricing | tokio |

## 1.4 Onde vive a parte interativa ao usuário

| Superfície | Binário/Crate | O que expõe |
|-----------|---------------|-------------|
| CLI usuário/agente | `arags` (`crates/arags-cli/src/cli/`, `dispatch/`, `output/`) | todos os comandos de dados; parsing em `cli/{root,commands}.rs`, roteamento em `dispatch/mod.rs` |
| Servidor operável | `arags-server` (`crates/arags-server/src/{main,admin,lifecycle}.rs`) | subcomandos `up`, `status`, `admin {create-refresh,revoke,prune-tokens,consolidate}`; imagem Docker única |
| Config do servidor | `server.toml` (host) lido via `ARAGS_SERVER_CONFIG` ou `/etc/arags/server.toml` | toda a configuração do data plane (sem `[llm]`) |
| Config do usuário | `~/.arags/arags.toml` (global) + `.arags.toml` (local, gitignored) | `[auth]`, `[llm]`, `[server]`, `[project]`, `[watch]`, `[volunteer]` |

## 1.5 Os quatro espaços de conhecimento (datasets)

Cada dataset tem espaço vetorial HNSW dedicado (384 dims, cosseno) + FTS5
quando lexical se aplica. **Nunca se misturam na escrita**; a unified query os
funde na leitura.

| | A — chunks | B — qa_cache | C — rlm_nodes | D — explorations |
|---|---|---|---|---|
| Unidade | pedaço de arquivo (~512 tokens, overlap 64) | pergunta→resposta digerida | sumário L1 arquivo/L2 tema/L3 projeto | mapa relacional orientado a objetivo |
| Origem | indexação mecânica | alguém perguntou e um LLM de cliente digeriu | voluntários com LLM local (bottom-up) | agente explorador investigou de verdade |
| Arquivo | `vectors.usearch` | `question_vectors.usearch` | `rlm_vectors.usearch` | `exploration_vectors.usearch` |
| Responde | "o que contém" | "já respondi isto" | "o que é este módulo" | "como as peças se conectam para X" |
| Fusão lexical+semântica | BM25+RRF | similaridade + tiers | FTS(`rlm_fts`)+vetorial, RRF normalizado | semântica + confidence composto |

### Unified Contextual Query (plan 023)

Uma única `search`/`query` devolve três seções:

1. **Results** — chunks verbatim (BM25 ⊕ semântica via RRF). Budget mínimo
   garantido; recebe o restante do orçamento de tokens.
2. **RLM Summaries** — sínteses aprovadas do dataset C; entram se score
   normalizado ≥ `[search].summary_min_score` e reivindicam até
   `[search].summary_ratio` (default 60%) do budget.
3. **Exploration Maps** — refs compactas dos mapas relevantes do dataset D,
   passando pelo pipeline de confiança completo (recheck de âncoras +
   grounding opcional + gate).

Campos são **aditivos no proto**: clientes antigos ignoram as seções novas.

### Pipeline de confiança (B, C, D)

- **Provenance por hash:** cada item derivado ancora `content_hash` dos chunks
  de origem. Re-index muda hash → entrada marcada `stale` (hit de QA vira
  MISS; nó RLM sai da busca até reprocessamento).
- **Âncoras de exploração:** cada path citado no mapa é âncora; qualquer mudança
  → `stale` com motivo. `verify_on_hit` opcional faz grounding lazy da
  afirmação-chave contra os vetores atuais (pega alucinação que hash não vê).
- **Feedback:** consumidores confirmam/contradizem mapas; contradições
  acumuladas aposentam (`contradiction_limit`).
- **Review gate:** com `[exploration].require_review=true`, mapas de
  não-admins nascem `pending_review` e só viram buscáveis após aprovação
  (`ReviewExploration` admin). O mesmo gate vale para nós RLM
  (`ReviewRlmNode`); voluntários admin auto-aprovam.

## 1.6 RLM — sumários recursivos distribuídos

```
chunks ──L1──▶ resumo do arquivo ──L2──▶ resumo do tema ──L3──▶ visão do projeto
        (voluntário + LLM local, lease exclusivo, submissão transacional)
```

- Agrupamento L2 determinístico por prefixo de path; tolerância progressiva
  (`[rlm].l2_tolerance=0.3`, `l3_tolerance=0.5`) evita reconstruir o global a
  cada ajuste trivial.
- Jobs ficam em fila (`rlm_jobs`); voluntários reclamam via `ClaimRlmJob`
  (lease default 500s), sintetizam com seu LLM e submetem via `CompleteRlmJob`
  — **transacional**: falha no meio devolve o job à fila sem perder o trabalho.

## 1.7 Auth (plan 018)

- Admin cria **refresh token** (validade 1 ano) com
  `arags-server admin create-refresh --username X --role admin|non_admin`.
- Cliente guarda em `~/.arags/arags.toml [auth]` (**global-only**) e troca por
  **sessão de curta duração** via `AuthRefresh`; o interceptor Bearer renova
  automaticamente.
- RPCs mutantes exigem `Bearer` válido; invalidações e reviews exigem role
  `Admin`. O admin CLI abre o SQLite diretamente dentro do container — não há
  caminho remoto de escalação de privilégio.

## 1.8 Layout de dados no servidor

```
$ARAGS_DATA_DIR (/data no container)
├── knowledge.db                  # SQLite WAL (buffers, chunks, FTS5, tokens,
│                                 #  qa_cache, rlm_*, explorations, history)
├── knowledge.db-wal              # WAL journal (checkpoint PASSIVE periódico)
├── vectors.usearch               # espaço A: chunks
├── question_vectors.usearch      # espaço B: perguntas QA
├── rlm_vectors.usearch           # espaço C: sumários RLM aprovados
└── exploration_vectors.usearch   # espaço D: mapas de exploração
```

Isolamento por projeto: cada projeto é um **buffer** (`buffers`, UUIDv7);
todas as tabelas escopam por `buffer_id`. Múltiplos agentes compartilham o
mesmo índice sem se enxergarem além do projeto.

## 1.9 Fluxos fim-a-fim

**Indexação:** cliente descobre arquivos (dot-paths fora, `.gitignore`
respeitado) → stream zstd do texto cru (`IndexProject`) → servidor chunka
(`[embedder].max_tokens/overlap_tokens`), embute em lotes (`batch_size`),
persiste em transações (`max_batch_size`) → pós-reindex marca staleness em
QA/RLM/explorações → enfileira jobs RLM L1.

**Busca (unified):** request → defaults de `[search]` aplicados → BM25 FTS5 +
ANN usearch → RRF → decay opcional (`decay_lambda`) → split de budget com
sumários C → anexo best-effort das explorações D → resposta tripla.

**QA-Cache:** `QueryWithCache(pergunta)` → embed no espaço B → hit exato/near-hit
(verifica `provenance_intact` antes de servir) → HIT devolve resposta pronta
(0 chamadas LLM); MISS devolve top-K chunks crus e o **cliente** digesta com o
LLM local, exibe e dispara `StoreAnswer` (fire-and-forget). Toda resposta tem
`cache_id` UUIDv7 estável → `GetAnswerById` é lookup determinístico 1:1
(anti-drift para sub-agentes).

## 1.10 Performance (targets e alavancas)

- Latência de busca típica ~21ms (< 100ms alvo); ingestão ~30s/10k arquivos.
- SQLite: page_size 8192, WAL, synchronous NORMAL, mmap 256MB, cache 64MB,
  busy_timeout 5s, hard_heap_limit 100MB.
- Escrita conciliada por pool (`pool_size`), lotes de transação
  (`max_batch_size`), checkpoint PASSIVE periódico (`flush_interval_ms`).
- Release: LTO, codegen-units=1, panic=abort, strip, mimalloc, musl estático no
  container (~109MB com modelo assado).

Continua em: [02-cli-arags.md](02-cli-arags.md) · [03-server-docker.md](03-server-docker.md)
