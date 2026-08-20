# TODO — arlm-search

> Busca híbrida: BM25 (FTS5) + Entity + Semantic (LanceDB) + RRF fusion.

## Status Atual

Busca BM25 e semantic funcionam. Falta integração com summaries e search tiers completos.

---

## Gaps Importantes (P1)

### 1. Sem busca na tabela summaries
- **Arquivo:** `src/hybrid.rs`
- **Problema:** `HybridSearch` não busca na tabela `summaries`. Plan 016 descreve busca dual-layer (raw + summaries).
- **Plano:** Plan 016 — Busca deve consultar tanto `chunks` quanto `summaries`.
- **Correção necessária:** Adicionar query FTS5 na tabela `summaries` e fusionar com resultados de chunks.

### 2. SearchResult.is_summary não populado
- **Arquivo:** `src/bm25.rs`, `src/semantic.rs`
- **Problema:** Resultados de busca nunca têm `is_summary: true` ou `summary: Some(...)`.
- **Plano:** Plan 016 — Resultados devem indicar se são sumários ou chunks brutos.
- **Correção necessária:** Marcar resultados vindos da tabela `summaries` com `is_summary: true`.

### 3. Search tiers incompletos
- **Arquivo:** `src/hybrid.rs`
- **Problema:** Tier 3 (LLM rerank) não está integrado — `HybridSearch` faz RRF mas não chama LLM para rerank.
- **Plano:** Plan 08 — Tier 3 deve usar LLM para reordenar resultados.
- **Correção necessária:** Adicionar chamada LLM no pipeline de busca quando tier = LlmRerank.

### 4. Entity search não integrada ao hybrid
- **Arquivo:** `src/hybrid.rs`
- **Problema:** `HybridSearch` usa BM25 + semantic mas não inclui `EntitySearch`.
- **Plano:** Plan 08 — Busca híbrida deve incluir entity recall.
- **Correção necessária:** Adicionar `EntitySearch` como fonte no RRF fusion.

---

## Gaps Menores (P2)

### 5. Context building limitado
- **Arquivo:** `src/context.rs`
- **Problema:** `ContextBuilder` formata resultados mas não tem lógica inteligente de seleção (ex: priorizar sumários sobre raw chunks).
- **Plano:** Plan 08 — Context building deve ser inteligente (selecionar chunk certo para cada parte do prompt).
- **Correção necessária:** Lógica de seleção baseada em token budget e relevância.

### 6. Decay não integrado ao search
- **Arquivo:** `src/decay.rs`
- **Problema:** `DecayConfig` existe mas não é aplicado nos resultados de busca.
- **Plano:** Plan 16 — Resultados devem ser re-ordenados por salience (decay).
- **Correção necessária:** Aplicar decay score após RRF fusion.

### 7. Sem cache de busca
- **Arquivo:** `src/`
- **Problema:** Buscas repetidas com mesma query fazem trabalho duplicado.
- **Plano:** N/A — otimização.
- **Correção necessária:** Adicionar cache LRU com TTL para queries frequentes.

---

## Referências

| Plano | Arquivo | Descrição |
|-------|---------|-----------|
| Plan 08 | `plan/08_*.md` | Busca híbrida completa, tiers, RRF |
| Plan 16 | `plan/16_*.md` | Dual-layer search, decay, entity recall |
