# arlm-search

Busca híbrida (BM25 + semântica) com fusão RRF para o arlm.

## Responsabilidades

- **BM25**: Busca textual via SQLite FTS5
- **Semantic**: Busca por similaridade via LanceDB
- **RRF**: Fusão Reciprocal Rank Fusion (k=60)
- **Context**: Montagem de contexto formatado para LLM

## Estrutura

```
src/
├── lib.rs          # Re-exports
├── types.rs        # SearchTier, SearchResult, HybridResult
├── bm25.rs         # Bm25Search com FTS5
├── semantic.rs     # SemanticSearch via LanceDB
├── hybrid.rs       # HybridSearch com RRF fusion
└── context.rs      # build_context (Prompt/JSON/Markdown)
```

## Tiers de Busca

| Tier | Nome | Latência | Como funciona |
|------|------|----------|---------------|
| 0 | `fts` | ~5ms | BM25 puro via FTS5 |
| 1 | `entity` | ~8ms | BM25 + entity RRF (padrão) |
| 2 | `vector` | ~21ms | BM25 + entity + vector RRF |
| 3 | `llm` | ~200ms | Tier 2 + LLM rerank (requer --llm) |

## Uso

```rust
use arlm_search::hybrid::HybridSearch;
use arlm_search::types::SearchOptions;

let search = HybridSearch::new(storage, embedder);

let results = search.search(
    "bug no login",
    &SearchOptions {
        tier: SearchTier::Entity,
        buffer_id: Some(1),
        top_k: 10,
        ..Default::default()
    }
)?;
```

## RRF Fusion

```rust
// Fusão Reciprocal Rank Fusion
// score = 1 / (k + rank) onde k=60
fn rrf_fuse(lists: Vec<Vec<(u64, f64)>>, k: f64) -> Vec<(u64, f64)> {
    // Combina scores de múltiplas listas
    // Retorna chunk_ids ordenados pelo score total
}
```

## Testes

```bash
cargo test -p arlm-search
```

26 testes cobrindo: BM25, semantic search, RRF fusion, context building.
