# arlm-search

Busca híbrida (BM25 + semântica) com fusão RRF para o arlm.

## Responsabilidades

- **BM25**: Busca textual via SQLite FTS5
- **Entity**: Busca por entidades regex (function names, imports)
- **Semantic**: Busca por similaridade via usearch (arlm-storage `VectorStore`)
- **RRF**: Fusão Reciprocal Rank Fusion (k=60)
- **QA-Cache similarity (plan 017)**: `qa_cache.rs` — `cosine_similarity` e
  `jaccard_similarity` (checagem secundária anti-falso-positivo: overlap de
  provenance entre a nova query e o cache).
- **Dual-layer (legacy)**: O `HybridSearch` também consulta a tabela `summaries`
  (FTS5 `summaries_fts`) e marca `is_summary` nos resultados. *Obs:* o servidor
  tornou-se LLM-free (plan 019), então a tabela `summaries` não é mais populada
  pelo servidor (não há sumarizador server-side).
- **Context**: Montagem de contexto formatado para LLM com token budget

> O servidor (`arlm-server`) é LLM-free. A busca híbrida (BM25 + semântica + RRF)
> roda inteiramente no servidor; o rerank/LLM só ocorre no cliente (`arlm-cli`) em
> `query -qa`/`persist`, via o LLM do usuário.

## Estrutura

```
src/
├── lib.rs          # Re-exports
├── types.rs        # SearchTier, SearchResult, HybridResult, ChunkWithText
├── bm25.rs         # Bm25Search com FTS5
├── entity.rs       # EntitySearch com regex
├── semantic.rs     # SemanticSearch via usearch (arlm-storage)
├── hybrid/
│   ├── mod.rs      # HybridSearch (RRF, decay, LLM rerank)
│   ├── rrf.rs      # Reciprocal Rank Fusion (matemática pura)
│   ├── fusion.rs   # apply_decay, search_fts, search_all (cross-project)
│   ├── search.rs   # Orquestração multi-tier async + dual-layer summaries
│   └── rerank.rs   # LLM rerank (Tier 3)
├── context.rs      # build_context, build_search_results, token budget
├── qa_cache.rs     # Similarity math p/ QA-Cache (cosine + Jaccard)
└── decay.rs        # Salience decay
```

## Tiers de Busca

| Tier | Nome | Latência | Como funciona |
|------|------|----------|---------------|
| 0 | `fts` | ~7ms | BM25 puro via FTS5 |
| 1 | `entity` | ~8ms | BM25 + entity RRF (padrão) |
| 2 | `vector` | ~21ms | BM25 + entity + vector RRF |
| 3 | `llm_rerank` | — | Tier 2 + LLM rerank — **não usado no servidor** (LLM-free); rerank, se aplicável, é feito no cliente |

## Funcionalidades

### Busca Cross-Project

```rust
// Busca em todos os projetos com RRF fusion
let results = hybrid.search_all("query", 10, &storage)?;
```

### Token Budget

```rust
// Contexto com limite de tokens
let ctx = build_context(&storage, &results, OutputFormat::Prompt, Some(8000))?;

// Resultados de busca com limite
let results = build_search_results(&storage, &results, Some(4000))?;
```

Truncamento inteligente: chunks são mantidos por score decrescente. O último chunk é truncado para caber no budget restante.

### RRF Fusion

```rust
// Fusão de múltiplas listas de resultado
let fused = HybridSearch::rrf_fuse(&[bm25_results, entity_results], 10, 60.0);
```

## Uso

```rust
use arlm_search::HybridSearch;
use arlm_search::types::{SearchOptions, SearchTier};

let bm25 = Bm25Search::new(&storage)?;
let entity = EntitySearch::new(storage.clone())?;
let hybrid = HybridSearch::new(bm25, Some(entity), None);

// BM25 puro
let results = hybrid.search_fts("query", buffer_id, 10, None)?;

// Tier entity (BM25 + entity RRF)
let options = SearchOptions { tier: SearchTier::Entity, top_k: 10 };
let results = hybrid.search("query", None, buffer_id, &options, None, Some(&storage)).await?;

// Cross-project
let all = hybrid.search_all("query", 10, &storage)?;
```

## FTS5 Otimizado

Tabela FTS5 usa `detail='column'` para ~40% menos espaço:
- Suporta: OR, AND, NOT, queries por coluna
- Não suporta: frases, NEAR (não necessário para BM25)

## Testes

```bash
cargo test -p arlm-search
```

56 testes cobrindo: BM25, entity search, semantic search, RRF fusion, dual-layer summaries, token budget, context building.
