# Plan 017: Semantic Query-Answer Cache (Digestão sob Demanda, no Client)

## Context

O summarizer hierárquico atual (`file → module → project`) é uma **agregação pré-computada e estática** que não faz sentido para desenvolvimento iterativo:

- `Summarizer::summarize` (`crates/arags-server/src/summarizer/engine.rs:53`) recarrega **todos** os chunks do buffer e re-sumariza **todo** o projeto a cada rodada de `summarize`, incondicionalmente.
- `insert_summary` (`crates/arags-server/src/store/summaries.rs:90`) é um `INSERT` puro, sem `ON CONFLICT` → rodar duas vezes **duplica** linhas.
- `estimate_incremental_cost` (`crates/arags-server/src/summarizer/cost.rs:55`) existe mas **nunca é chamado** → o caminho incremental foi planejado e não conectado.
- O servidor carrega `LlmBackend` **só** para sumarizar (`engine.rs:243`), duplicando o LLM que o agente consumidor (Continue/Aider/Cline) já possui.

Este plano substitui a hierarquia estática por um **cache semântico de respostas digeridas na hora da query**, com duas decisões de arquitetura:

1. **A digestão (síntese por LLM) roda no CLIENT**, usando o LLM do próprio usuário (configurado em `~/.arags/config.toml` via `arags-llm`). O servidor **não aciona nenhum LLM** — ele apenas armazena e processa **deterministicamente** (embedding, FTS, hashes, RRF).
2. **O armazenamento vetorial usa `usearch`** (HNSW single-file, mais enxuto e plenamente capaz para o escopo; o projeto migrou do LanceDB para o usearch).

Isso descentraliza custo/modelo, mantém o servidor enxuto e é coerente com o princípio "agent-agnostic" do projeto. A digestão sob demanda é a materialização do padrão do `rlm_guide` (Sub-LLM digere chunks *pela necessidade de informação*), agora no client.

Este plano **já incorpora as correções** discutidas:

1. **Versionamento / invalidação** por mudança de código (senão o cache mente após refatorações).
2. **Resolução do cosseno** (similaridade de pergunta é proxy fraco; evitar falso-positivo tipo "login" vs "logout").
3. **Thresholds flexíveis/configuráveis**.
4. **Eviction razoável**.
5. **Provenance** obrigatória (chunk_ids que geraram a resposta).
6. **5 pontos de atenção** do design client-side (ver seção dedicada).

---

## Goals

- Servidor **sem LLM**: só embedding + SQLite + `usearch` + ops determinísticas.
- Pagar 1 digestão por **pergunta nova**, no client do usuário; amortizar nas repetidas/próximas.
- Redução de contexto **monótona e limitada** (`trechos_na_resposta ≤ trechos_novelos`).
- Respostas em cache **nunca stale** após mudança de código (invalidação por hash de chunk).
- Falso-positivo de cache minimizado (checagem secundária além do cosseno de pergunta).
- Tudo configurável (limiares, dimensões, política de eviction) sem rebuild.

## Non-goals

- Não elimina a busca híbrida (BM25 + semântica) — o cache **consome** o resultado dela.
- Não roda LLM no servidor (removido o `LlmBackend` de summarization do server).
- Não substitui embeddings de chunk; usa um **espaço vetorial separado** (`usearch`) para perguntas.

---

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────┐
│  Client (arags-cli) — USA O LLM DO USUÁRIO (config.toml)        │
│                                                                │
│  1. QueryWithCache(pergunta, project)  ──gRPC──►  Server       │
│  6. ◄── HIT: answer+provenance  (0 LLM no client)              │
│       ◄── MISS: top-K chunks crus                             │
│  4. [MISS] 1 chamada LLM sintetiza top-K → answer             │
│  5. exibe answer ao usuário (UX imediata)                      │
│  7. StoreAnswer(answer, provenance, source_hashes) fire-forget│
└───────────────────────────┬──────────────────────────────────┘
                            │
                            ▼  (server = DETERMINÍSTICO, sem LLM)
┌──────────────────────────────────────────────────────────────┐
│  arags-server                                                   │
│  • embed(query) + HybridSearch (BM25+entity+vector) → top-K    │
│  • embed(pergunta) espaço B (usearch question_vectors)         │
│  • lookup cache (cosseno + checagem secundária) → tier         │
│  • StoreAnswer: FTS do answer + grava qa_cache + marca stale   │
│  • RRF funde cache + chunks                                    │
└──────────────────────────────────────────────────────────────┘
```

---

## Data Model

### Tabela `qa_cache` (SQLite, `arags-storage`)

| Coluna | Tipo | Papel |
|---|---|---|
| `id` | INTEGER PK | |
| `cache_id` | TEXT (UUIDv7, unique) | **ID estável da resposta**, propagável p/ sub-agentes (anti-drift) |
| `buffer_id` | INTEGER | scoping por projeto/buffer |
| `project` | TEXT | redundância p/ lookup rápido |
| `question_text` | TEXT | texto original da pergunta |
| `question_hash` | TEXT | hit exato (mesma pergunta, mesmo buffer) |
| `answer_text` | TEXT | resposta digerida (cache, vinda do client) |
| `source_chunk_ids` | TEXT (JSON) | **provenance**: chunk_ids usados |
| `source_hashes` | TEXT (JSON) | **invalidation**: hashes dos chunks |
| `model` | TEXT | **metadado** do LLM que sintetizou (não bloqueia reuso) |
| `confidence` | REAL | decai se algum chunk ficar stale |
| `tier_snapshot` | TEXT (JSON) | limiares usados (reprodutibilidade) |
| `token_count` | INTEGER | custo da resposta |
| `access_count` | INTEGER | p/ eviction ponderado |
| `created_at` | INTEGER | epoch ms |
| `last_accessed_at` | INTEGER | epoch ms (LRU) |
| `stale` | INTEGER (bool) | marcado por invalidação |
| `invalidated_at` | INTEGER | epoch ms da invalidação manual (audit) |
| `invalidated_by` | TEXT | quem invalidou (audit) |
| `invalidated_reason` | TEXT | motivo (ex: alucinação) |

FTS5 opcional sobre `question_text` (e/ou `answer_text`) para busca lexical de cache.

### Espaço vetorial separado (perguntas) — `usearch`

- **Novo** índice `question_vectors` (`usearch`, HNSW single-file), **distinto** do `chunk_vectors`.
- Dimensões configuráveis (`question_vector_dims`), reusando o embedder do servidor com prefixo de task (`search_query: `) — mas **nunca** misturado ao espaço de chunk.
- Chave estrangeira lógica `question_vectors.id → qa_cache.id`.

## Query Flow (client-centric, com thresholds configuráveis)

Parâmetros default (todos sobrescrevíveis em config):

| Parâmetro | Default | Significado |
|---|---|---|
| `novel_k` | 20 | trechos digeridos numa pergunta nova (client) |
| `provenance_k` | 5 | trechos devolvidos junto da resposta em cache |
| `sim_high` | 0.90 | acima disso → reaproveita resposta + re-digest leve |
| `sim_floor` | 0.40 | abaixo disso → trata como nova (digest completo) |
| `tier_steps` | [0.90, 0.80, 0.70, 0.60, 0.50] | fronteiras de widen |

Fluxo no client:

1. Cliente chama `QueryWithCache(pergunta, project)` no server.
2. Server (determinístico): embed(query) + HybridSearch → top-K; embed(pergunta) espaço B; lookup cache → decide `s` (similaridade) e tier.
3. Server responde:
   - **HIT exato / tier alto**: `answer_text` + top `provenance_k` trechos de provenance. Cliente **não chama LLM** (0 custo).
   - **MISS / tier baixo**: top-K chunks crus. Cliente faz **1 chamada LLM** sintetizando → `answer`, exibe ao usuário, e dispara `StoreAnswer` (fire-and-forget).
4. Comportamento por tier (`s` após checagem secundária):
   - `s ≥ sim_high`: re-digere 10 trechos, devolve `answer + 5`, atualiza cache.
   - `0.80 ≥ s > sim_high`… descendo: digere 10→15, devolve 5→10 conforme `tier_steps`.
   - `s < sim_floor`: MISS → digest completo (`novel_k`=20), persiste nova entrada.
5. **Invariante de redução:** `trechos_na_resposta ≤ trechos_novelos (20)` em todos os tiers.

---

## Escopo por projeto e reserve lock (multi-dev / multi-projeto)

Cenário alvo: vários devs no **mesmo** projeto; 1 dev em **vários** projetos. Regras:

- **Conhecimento é escopado no PROJETO** (`buffer_id`/`project`). Não há compartilhamento de cache entre projetos diferentes.
- **Mesma pergunta, mesmo projeto, devs diferentes → preserva-se o 1º.** O primeiro digest vence; os demais reutilizam a entrada (hit). A chave de cache é `(project, question_hash)`.
- **Mesma pergunta, projetos diferentes → regra independente.** Cada projeto tem sua própria entrada (próprio `cache_id`); sem cross-contaminação.
- **Reserve lock:** no MISS, o server reserva `(project, question_hash)` *antes* de disparar a digestão no client, para que requisições concorrentes idênticas (mesmo projeto) não geram digests duplicados — elas aguardam/retornam o mesmo `cache_id`. O lock é por projeto, então não bloqueia perguntas em outros projetos.
- `GetAnswerById` também é escopado por projeto (`cache_id` + `project`/`buffer_id`).

---

## Similarity Resolution (correção do cosseno)

Cosseno de pergunta é necessário mas **insuficiente** (falso-positivo: "onde é login?" ≈ "onde é logout?"). Pipeline (lado server, determinístico exceto a comparação de vetores):

1. `embed(pergunta)` no espaço B (`usearch`) → busca k candidatos (`sim ≥ sim_floor`).
2. **Checagem secundária** para o melhor candidato:
   - **Overlap de snippet-set:** Jaccard entre os `top-K` chunks da nova query e os `source_chunk_ids` do cache. Se `J ≥ 0.5` → mesmo *information need* confirmado.
   - **Ou** re-embed da `answer_text` do cache vs embed da nova pergunta; se cosseno alto, confirma.
3. Só se (1) **e** (2) passarem é que conta como HIT/tier. Caso contrário → MISS (digest novo no client).

---

## Invalidation (correção de staleness)

Cada entrada guarda `source_hashes` (hash de conteúdo de cada chunk usado).

- No `reindex`/`lifecycle` (`crates/arags-server/src/lifecycle.rs`), após atualizar chunks, computa-se o diff de hashes por buffer (determinístico, server-side).
- Toda entrada de `qa_cache` cujo `source_hashes` contiver **qualquer** chunk que mudou/sumiu → marcada `stale=1` e `confidence=0`.
- Hit em entrada stale força **re-digest completo** no client (trata como MISS), regenerando a resposta com chunks frescos.
- Garante que, após refatoração de 5k linhas, respostas afetadas são automaticamente invalidadas.

---

## Eviction (política razoável)

- **LRU ponderado**: `score = access_count / (1 + (now - last_accessed_at)/λ)`; remove os piores quando `count > max_entries_per_project`.
- **TTL opcional**: entradas com `last_accessed_at` mais antigo que `cache_ttl_ms` expiram.
- Roda em background (worker) ou no acesso, com batch `DELETE`.

---

## Provenance

- `source_chunk_ids` + `file_path` devolvidos junto da resposta para o agente consumidor poder (a) fundamentar e (b) permitir invalidação.
- No `build_context`/`build_search_results` (`crates/arags-search/src/context.rs`), entradas de cache são resolvidas como `is_cache_answer` para montar o prompt.

---

## Resposta com ID estável e lookup direto (anti-drift)

Toda resposta servida recebe um **`cache_id` (UUIDv7)** no momento da criação (no `StoreAnswer`, server-side, via `uuid::Uuid::now_v7()` — padrão do projeto, ver `crates/arags-server/src/grpc/summarize.rs:27`). O `cache_id` é **distinto do `id` (rowid)** e é o identificador estável da resposta.

### Por quê

Um agente orquestrador (Root LLM) faz a consulta, recebe a resposta **+ `cache_id`**, e pode repassar esse ID a um **sub-agente**. O sub-agente aciona o lookup direto e recebe **exatamente o mesmo contexto (answer + provenance)**, sem re-digestão e sem deriva semântica (drift) causada por nova busca híbrida ou nova síntese LLM.

### Regras

1. **Todo served response inclui `cache_id`**, independente do formato de output (Prompt/Json/Markdown) e independente de ser HIT ou MISS (no MISS, o `StoreAnswer` retorna o `cache_id` recém-criado ao client).
2. **Lookup direto (`GetAnswerById`)**: RPC que recebe `cache_id` (+ `buffer_id`/`project` p/ escopo) e devolve `answer_text` + `source_chunk_ids` + `cache_id` **1:1**, **sem** tocar em indexação, embedding, busca híbrida ou LLM. É o caminho de "contexto determinístico e reproduzível".
3. O CLI expõe esse lookup direto (ex: `arags query --cache-id <uuidv7>`), que **não passa** pelo sistema de index/retrieval — apenas retorna a resposta salva.
4. `GetAnswerById` também serve para o orquestrador re-obter sua própria resposta sem custo.

---

## Invalidação manual (reset de resposta e cadeia de erros)

Cenário: um dev alucina ao sumarizar "como proteger senhas?"; a resposta vira bug conceitual. Por ser recente/relevante, vários devs fazem perguntas próximas e o fluxo leva ao mesmo bug. Um sênior deve poder **forçar o reset** daquela resposta e das próximas a ela, invalidando a cadeia de erros.

### Mecanismo

- RPC `InvalidateCache`:
  - `cache_id` (alvo).
  - `mode`: `Stale` (soft — marca `stale=1`, `confidence=0`, preenche `invalidated_at/by/reason`) ou `Delete` (hard — remove a linha de `qa_cache` + o vetor em `question_vectors`).
  - `similarity_radius`: `Option<f32>`. Se presente, além do alvo, **invalida também as entradas cuja pergunta está dentro do raio de similaridade** no espaço `question_vectors` (reusa o próprio índice `usearch` para achar vizinhos — a "cadeia de erros"). Default sugerido `0.85`.
- Após `Stale`, a próxima query que der hit naquela (ou vizinha) entrada **força re-digest** (tratada como MISS) → resposta fresca, corrigindo o bug sem apagar o histórico (auditável).
- `Delete` remove de vez (sem trilha, exceto log do server).

### CLI

`arags cache invalidate --cache-id <uuidv7> [--radius 0.85] [--delete] [--reason "alucinacao"]`

### Privilégio

Em time fechado, qualquer client pode invalidar. Em multi-user restrito (plan 015), deve ser **opção privilegiada** (role sênior) — conecta com o ponto de atenção 3 (trust/poisoning).

---

## Resolução dos 5 pontos de atenção (design client-side)

| # | Ponto | Resolução planejada |
|---|-------|---------------------|
| 1 | **Provenance no store RPC** | `StoreAnswer` (server) recebe `source_chunk_ids` + `source_hashes` do client e persiste em `qa_cache`; sem isso a invalidação (item 3) quebra. Implementado em `grpc/query_cache.rs` + `store/qa_cache.rs`. |
| 2 | **Race de digest duplicado** | Resolvido por **reserve lock** no server, chave `(project, question_hash)`: no MISS o server reserva antes de disparar a digestão, e requisições concorrentes idênticas (mesmo projeto) reutilizam o mesmo `cache_id`. O lock é por projeto → não bloqueia outros projetos. Cross-project a pergunta é independente (regra própria). |
| 3 | **Trust / poisoning em multi-user** | Server armazena cegamente o que o client manda → risco de envenenamento em multi-user. Single-user local: OK. Multi-user (roadmap plan 015): exige auth + validação/quotas. Fora de escopo deste plano; referenciado. |
| 4 | **Model-specific** | **Removido como bloqueador.** `qa_cache.model` guarda metadado do LLM que sintetizou, mas **não impede** reuso da informação entre os modelos da equipe — a resposta é servida a qualquer client independente do modelo. |
| 5 | **Fire-and-forget no envio ao server** | Cliente exibe a resposta imediatamente; o `StoreAnswer` roda em background/`spawn_blocking` sem bloquear a UX do usuário. |

---

## Where to Implement

| Componente | Crate | Arquivo(s) |
|---|---|---|
| Schema `qa_cache` + FTS + `usearch` `question_vectors` + `evict()` | `arags-storage` | `src/store/qa_cache.rs` (novo) + migração |
| Embed de pergunta (prefixo task, espaço B) | `arags-embedding` | `embedder/mod.rs` (`embed_query`) |
| Lookup + checagem secundária | `arags-search` | `src/qa_cache.rs` (novo) |
| Engine de widening adaptativo | `arags-core` | `src/qa_cache/mod.rs` (novo) |
| **Digest-once (LLM) — CLIENT** | `arags-cli` (via `arags-llm` + `config.toml`) | `src/commands/query_cache.rs` (novo) |
| Invalidation no reindex (determinístico) | `arags-server` | `lifecycle.rs` + hook pós-index |
| Eviction | `arags-storage` | `src/store/qa_cache.rs` (`evict`) |
| `StoreAnswer` RPC (server, determinístico) | `arags-proto`, `arags-server` | `proto/arags.proto`, `grpc/query_cache.rs` |
| Config (thresholds/dims/eviction) | `arags-server` + `arags-cli` | `config.rs` (`QaCacheConfig`) + user LLM config |
| `GetAnswerById` (lookup direto 1:1, anti-drift) | `arags-proto`, `arags-server`, `arags-cli` | `proto/arags.proto`, `grpc/query_cache.rs`, `cli/commands.rs` (`--cache-id`) |
| `InvalidateCache` (reset manual: single + cluster por raio) | `arags-proto`, `arags-server`, `arags-cli` | `proto/arags.proto`, `grpc/query_cache.rs` (Stale/Delete + `similarity_radius` via `usearch`), `cli/commands.rs` (`cache invalidate`) |
| Testes/bench | `tests/`, `benches/` | `qa_cache_test.rs`, `qa_cache_bench.rs` |

> Nota: o servidor **não** precisa de `LlmBackend` para este fluxo. O `arags-server/src/summarizer/` torna-se obsoleto para o cache (pode ser mantido como legado ou removido).

---

## Implementation Steps (milestones)

1. **Storage**: `qa_cache` + `usearch question_vectors` + migração + `evict()`.
2. **Embedding**: `embed_query` com prefixo de task no espaço B.
3. **Lookup**: busca por cosseno + checagem secundária (Jaccard/overlap).
4. **Adaptive engine**: mapear `s` → tier → `(digest_k, provenance_k)`.
5. **Digest-once (CLIENT)**: síntese LLM de top-K (usa `arags-llm` + config.toml) → `answer` + provenance + hashes; exibe e dispara `StoreAnswer` fire-and-forget.
6. **Invalidation**: hook em `lifecycle` pós-reindex marcando `stale`.
7. **Eviction**: LRU ponderado em background.
8. **Config**: `QaCacheConfig` + LLM do usuário no client.
9. **Wiring**: RPC `QueryWithCache` + `StoreAnswer` (server) + cliente CLI orquestrando.
10. **Tests/Bench**: corretude de hit/miss/tier, invalidação, eviction, redução de contexto.

---

## Testing & Benchmarks

- `test_qa_cache_exact_hit_zero_llm_calls` (client não chama LLM).
- `test_qa_cache_near_hit_widens_context` (verifica tier → k).
- `test_qa_cache_false_positive_login_vs_logout` (checagem secundária bloqueia).
- `test_qa_cache_invalidated_after_chunk_hash_change` (stale → re-digest no client).
- `test_qa_cache_eviction_lru` (cap respeitado).
- `test_qa_cache_store_fire_and_forget` (resposta exibida antes do save completar).
- `test_qa_cache_id_emitted_in_all_formats` (Prompt/Json/Markdown incluem `cache_id`).
- `test_qa_cache_get_by_id_returns_identical_1to1` (lookup direto ignora index/busca/LLM e devolve exatamente a mesma resposta+provenance).
- `test_qa_cache_scoped_per_project` (mesma pergunta em projetos A e B gera entradas independentes, `cache_id` distintos, sem cross-contaminação).
- `test_qa_cache_reserve_lock_dedupes_same_project` (MISS concorrente idêntico no mesmo projeto → 1 digest, mesmo `cache_id` p/ todos; lock não afeta outros projetos).
- `test_qa_cache_invalidate_single_marks_stale_forces_redigest` (Stale faz re-digest na próxima query).
- `test_qa_cache_invalidate_cluster_by_radius` (raio invalida perguntas vizinhas no espaço `question_vectors`).
- `test_qa_cache_invalidate_delete_removes_entry_and_vector`.
- Bench: latência de cache hit vs digest novo; redução de tokens na resposta.

---

## Risks

| Risco | Mitigação |
|---|---|
| Embed de pergunta com prefixo errado polui retrieval | usar espaço B dedicado + prefixo `search_query:` validado |
| Checagem secundária cara | só roda sobre k candidatos (≤10) |
| Eviction agressiva apaga cache útil | peso por `access_count` + TTL generoso default |
| Resposta cacheada grande em tokens | `token_count` no budget do contexto |
| Drift do embedder (troca de modelo) | versionar `embedding_model` na entrada; invalidar em troca |
| Poisoning em multi-user | auth/validação (plan 015); single-user local OK |
| Race de digest duplicado | custo client-side aceitável; reserve lock opcional |

