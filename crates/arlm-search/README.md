# arlm-search

Busca híbrida (BM25 + semântica) com fusão RRF para o arlm.

## Responsabilidades

- **BM25**: Busca textual via SQLite FTS5
- **Entity**: Busca por entidades regex (function names, imports)
- **Semantic**: Busca por similaridade via LanceDB
- **RRF**: Fusão Reciprocal Rank Fusion (k=60)
- **Context**: Montagem de contexto formatado para LLM com token budget

## Estrutura

```
src/
├── lib.rs          # Re-exports
├── types.rs        # SearchTier, SearchResult, HybridResult
├── bm25.rs         # Bm25Search com FTS5
├── entity.rs       # EntitySearch com regex
├── semantic.rs     # SemanticSearch via LanceDB
├── hybrid.rs       # HybridSearch com RRF, search_all()
├── context.rs      # build_context, build_search_results, token budget
└── decay.rs        # Salience decay
```

## Tiers de Busca

| Tier | Nome | Latência | Como funciona |
|------|------|----------|---------------|
| 0 | `fts` | ~7ms | BM25 puro via FTS5 |
| 1 | `entity` | ~8ms | BM25 + entity RRF (padrão) |
| 2 | `vector` | ~21ms | BM25 + entity + vector RRF |
| 3 | `llm_rerank` | ~200ms | Tier 2 + LLM rerank |

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

32 testes cobrindo: BM25, entity search, semantic search, RRF fusion, token budget, context building.
