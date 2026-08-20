# TODO — arlm-core

> Engine RLM recursivo: planner → solver → synthesizer.
> Contém o loop principal, guardrails, budget, eventos, cache, e tipos de domínio.

## Status Atual

O engine RLM funciona em modo local (CLI). Todos os 10 gaps (#1–#10) estão **Concluído**
— veja as notas individuais abaixo. Integração com memória/busca é feita via traits
`MemoryProvider`/`CodeSearch` definidos em `arlm-core`, com backend concreto injetado por
outras crates (comportamento no-op/honesto quando `None`).

---

## Gaps Críticos (P0)

### 1. SearchCodeTool é placeholder — **Concluído**
- **Arquivo:** `src/tools.rs`
- **Problema:** Tool retorna resultado fake — `"Placeholder: in real implementation, this would call arlm-search"`.
- **Plano:** Plan 016 — Tools devem ser executáveis de verdade.
- **Status:** `SearchCodeTool` agora usa `Option<Arc<dyn CodeSearch>>` (trait definido em `arlm-core`); sem backend retorna mensagem honesta `"search_code not configured: no code-search backend provided"`.

### 2. Sem injeção de contexto da memória — **Concluído**
- **Arquivo:** `src/solver.rs` (`solve_task`, `solve_task_repl`)
- **Problema:** Solver não recebe `MemoryEngine`, não chama `memory.context()` antes de cada chamada LLM.
- **Plano:** Plan 05 — Solver deve buscar contexto relevante da memória antes de cada LLM call.
- **Status:** `MemoryProvider` trait em `src/memory.rs`; `solve_task`/`solve_task_repl` recebem `Option<Arc<dyn MemoryProvider>>` e prependem o contexto ao system prompt via `build_memory_context`.

### 3. Sem persistência de trajectory — **Concluído**
- **Arquivo:** `src/engine/mod.rs` (`run_rlm_engine_with_events`)
- **Problema:** Após cada run, trajectory não é salva na memória.
- **Plano:** Plan 13 — Após cada run, persistir trajectory via `memory.save_trajectory()`.
- **Status:** `MemoryProvider::save_trajectory` é chamado ao final de `run_rlm_engine_with_events` quando `memory` é `Some`; falhas são logadas via `tracing::warn` (no-op quando `None`).

---

## Gaps Importantes (P1)

### 4. Synthesizer sem compaction baseada em tokens — **Concluído**
- **Arquivo:** `src/synthesizer.rs` (`build_children_block`, `compact_children_if_needed`)
- **Problema:** Sempre passa todos os filhos brutos, sem limitar por tokens.
- **Plano:** Plan 13 — Synthesizer deve compactar filhos quando excederem 85% do contexto, sumarizando os mais antigos via LLM.
- **Status:** `compact_children_if_needed` sumariza via LLM os filhos mais antigos quando os tokens acumulados ultrapassam 85% do limite do modelo (`CHILD_COMPACTION_CONTEXT_FRACTION`).

### 5. CompactionPolicy não utilizada — **Concluído**
- **Arquivo:** `src/synthesizer.rs`, `src/types/enums.rs` (`CompactionPolicy`)
- **Problema:** Campo existe mas nunca é lido pelo engine.
- **Plano:** Plan 13 — `CompactionPolicy` deve guiar quando e como compactar.
- **Status:** `compact_children_if_needed` respeita `policy.enabled` e `policy.max_child_tokens` (`threshold = min(85% context, max_child_tokens)`).

### 6. RootCompactor é rudimentar — **Concluído**
- **Arquivo:** `src/engine/compactor.rs`
- **Problema:** Apenas trunca para 1000 chars e mantém 10 sumários — sem sumarização via LLM.
- **Plano:** Plan 13 — Root compaction deve usar LLM para sumarizar outputs acumulados.
- **Status:** Adicionado `RootCompactor::summarize_with_llm` que usa o LLM para sumarizar; `get_summary` (não-LLM) permanece como fallback.

### 7. Sem EventSink dedicado — **Concluído**
- **Arquivo:** `src/events.rs`
- **Problema:** Usa `Arc<EventBus>` diretamente. Plan 14 descreve `EventSink` como wrapper thread-safe.
- **Plano:** Plan 14 — `EventSink` com `emit()` que garante thread-safety.
- **Status:** `EventSink` (wrapper `Arc<EventBus>`) com `emit`/subscribe/From impls; `EventBus` API intacta (apenas aditivo).

---

## Gaps Menores (P2)

### 8. SamplingArgs sem campo seed — **Concluído**
- **Arquivo:** `src/sampling.rs`
- **Problema:** `SamplingArgs` não tem `seed: Option<u64>` para reprodutibilidade.
- **Plano:** Plan 12 — `seed` para sampling determinístico.
- **Status:** `seed: Option<u64>` adicionado com `with_seed`/`seed()`; preservado em `SamplingArgs` para backends que suportam seeding (propagação no `apply_to_request` quando o wire suportar).

### 9. Token counter usa heuristic de palavras — **Concluído**
- **Arquivo:** `src/token_counter.rs`
- **Problema:** Contagem de tokens é baseada em `split_whitespace()` — não é precisa.
- **Plano:** Plan 13 — Token counting deve usar tokenizer real do modelo (ou approximação melhor).
- **Status:** Heurística melhorada (~4 chars/token + surcharge de pontuação ASCII), sem novas dependências pesadas; API estável.

### 10. Cache não tem invalidação por dependências — **Concluído**
- **Arquivo:** `src/cache.rs`
- **Problema:** Cache usa TTL + LRU mas não invalida quando dependências mudam.
- **Plano:** Plan 13 — Cache deve invalidar quando inputs mudam.
- **Status:** `CacheEntry` ganha `dep_key`/`dep_version`; `get_dep`/`put_dep`/`invalidate_dep` fazem invalidação por dependência, mantendo TTL/LRU compatíveis.

---

## Referências

| Plano | Arquivo | Descrição |
|-------|---------|-----------|
| Plan 05 | `plan/05_*.md` | Engine RLM, planner/solver/synthesizer, context injection |
| Plan 12 | `plan/12_*.md` | Budget, pricing, model fallback |
| Plan 13 | `plan/13_*.md` | Context management, compaction, trajectory persistence |
| Plan 14 | `plan/14_*.md` | Observabilidade, EventBus, LiveTree |
