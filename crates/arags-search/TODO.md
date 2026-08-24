# TODO — arags-search

> Busca híbrida: BM25 (FTS5) + Entity + Semantic (usearch) + RRF fusion + dual-layer summaries.

## Status Atual

Busca BM25, entity, semantic e rerank LLM (Tier 3) funcionam. Decay de saliência está
integrado na fusão. **Busca dual-layer em summaries implementada** (gap #1/#2): o
`HybridSearch` consulta a tabela `summaries` (FTS5 `summaries_fts`) e marca
`is_summary` nos resultados.

> **OBSOLETO (pós plano 019):** o servidor é LLM-free; o rerank por LLM (Tier 3)
> não é usado server-side e a tabela `summaries` não é mais populada (sem
> sumarizador server-side). Veja `plan/019-cli-consolidation.md`.

---

## Gaps — Resolvidos nesta refatoração

| # | Gap | Estado | Onde |
|---|-----|--------|------|
| 1 | Busca na tabela `summaries` | ✅ `HybridSearch::search`/`search_all` fundem summaries via RRF | `hybrid/search.rs`, `hybrid/fusion.rs`, `arags-storage/summaries.rs` |
| 2 | `is_summary`/`summary` nos resultados | ✅ `HybridResult.is_summary` + `SearchResult.is_summary`/`summary_scope` | `types.rs`, `context.rs` |
| 3 | Tier 3 (LLM rerank) | ✅ Já integrado (`rerank_with_llm` em Tier 3) | `hybrid/rerank.rs`, `hybrid/search.rs` |
| 4 | Entity integrado ao hybrid | ✅ `EntitySearch` é fonte no RRF (Tier 1+) | `hybrid/search.rs` |
| 6 | Decay integrado ao search | ✅ `apply_decay` aplicado após fusão | `hybrid/fusion.rs` |

## Gaps — Menores / Fora de escopo

### 5. Context building limitado
- **Arquivo:** `src/context.rs`
- **Estado:** Funcional (token budget + seleção por score). Melhorias (priorizar
  summaries sobre raw chunks no prompt) são incrementais e não bloqueiam.

### 7. Sem cache de busca
- **Arquivo:** `src/`
- **Estado:** Otimização futura (LRU+TTL). Não implementada; fora do escopo desta refatoração.

---

## Referências

| Plano | Arquivo | Descrição |
|-------|---------|-----------|
| Plan 08 | `plan/08_*.md` | Busca híbrida completa, tiers, RRF |
| Plan 16 | `plan/16_*.md` | Dual-layer search, decay, entity recall |
