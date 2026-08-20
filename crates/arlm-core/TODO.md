# TODO — arlm-core

> Engine RLM recursivo: planner → solver → synthesizer.
> Contém o loop principal, guardrails, budget, eventos, cache, e tipos de domínio.

## Status Atual

O engine RLM funciona em modo local (CLI). Falta integração com memória, persistência de trajectory, e compaction no synthesizer.

---

## Gaps Críticos (P0)

### 1. SearchCodeTool é placeholder
- **Arquivo:** `src/types.rs:228`
- **Problema:** Tool retorna resultado fake — `"Placeholder: in real implementation, this would call arlm-search"`.
- **Plano:** Plan 016 — Tools devem ser executáveis de verdade.
- **Correção necessária:** Integrar com `arlm-search` para busca real, ou tornar `ExecutableTool` que recebe referência ao search backend.

### 2. Sem injeção de contexto da memória
- **Arquivo:** `src/solver.rs` (função `solve_task`)
- **Problema:** Solver não recebe `MemoryEngine`, não chama `memory.context()` antes de cada chamada LLM.
- **Plano:** Plan 05 — Solver deve buscar contexto relevante da memória antes de cada LLM call.
- **Correção necessária:** Adicionar parâmetro `memory: Option<&MemoryEngine>` ao `solve_task` e chamar `memory.context(task)`.

### 3. Sem persistência de trajectory
- **Arquivo:** `src/engine.rs` (função `run_rlm_engine`)
- **Problema:** Após cada run, trajectory não é salva na memória.
- **Plano:** Plan 13 — Após cada run, persistir trajectory via `memory.save_trajectory()`.
- **Correção necessária:** Ao final de `run_rlm_engine`, chamar `memory.save_trajectory(input, result)`.

---

## Gaps Importantes (P1)

### 4. Synthesizer sem compaction baseada em tokens
- **Arquivo:** `src/synthesizer.rs` (função `build_children_block`)
- **Problema:** Sempre passa todos os filhos brutos, sem limitar por tokens.
- **Plano:** Plan 13 — Synthesizer deve compactar filhos quando excederem 85% do contexto, sumarizando os mais antigos via LLM.
- **Correção necessária:** Adicionar lógica de contagem de tokens e compaction dinâmica.

### 5. CompactionPolicy não utilizada
- **Arquivo:** `src/types.rs` (campo `StartRunInput.compaction`)
- **Problema:** Campo existe mas nunca é lido pelo engine.
- **Plano:** Plan 13 — `CompactionPolicy` deve guiar quando e como compactar.
- **Correção necessária:** Engine deve respeitar `input.compaction` ao decidir compactar.

### 6. RootCompactor é rudimentar
- **Arquivo:** `src/engine.rs` (struct `RootCompactor`)
- **Problema:** Apenas trunca para 1000 chars e mantém 10 sumários — sem sumarização via LLM.
- **Plano:** Plan 13 — Root compaction deve usar LLM para sumarizar outputs acumulados.
- **Correção necessária:** Integrar chamada LLM no `RootCompactor::get_summary()`.

### 7. Sem EventSink dedicado
- **Arquivo:** `src/events.rs`
- **Problema:** Usa `Arc<EventBus>` diretamente. Plan 14 descreve `EventSink` como wrapper thread-safe.
- **Plano:** Plan 14 — `EventSink` com `emit()` que garante thread-safety.
- **Correção necessária:** Criar wrapper `EventSink` ou confirmar que `Arc<EventBus>` é suficiente.

---

## Gaps Menores (P2)

### 8. SamplingArgs sem campo seed
- **Arquivo:** `src/sampling.rs`
- **Problema:** `SamplingArgs` não tem `seed: Option<u64>` para reprodutibilidade.
- **Plano:** Plan 12 — `seed` para sampling determinístico.
- **Correção necessária:** Adicionar campo `seed` e propagar para LLM calls.

### 9. Token counter usa heuristic de palavras
- **Arquivo:** `src/token_counter.rs`
- **Problema:** Contagem de tokens é baseada em `split_whitespace()` — não é preciso.
- **Plano:** Plan 13 — Token counting deve usar tokenizer real do modelo (ou approximação melhor).
- **Correção necessária:** Considerar usar `tiktoken-rs` ou similar.

### 10. Cache não tem invalidação por dependências
- **Arquivo:** `src/cache.rs`
- **Problema:** Cache usa TTL + LRU mas não invalida quando dependências mudam.
- **Plano:** Plan 13 — Cache deve invalidar quando inputs mudam.
- **Correção necessária:** Hash de dependências (arquivos, config) para invalidação inteligente.

---

## Referências

| Plano | Arquivo | Descrição |
|-------|---------|-----------|
| Plan 05 | `plan/05_*.md` | Engine RLM, planner/solver/synthesizer, context injection |
| Plan 12 | `plan/12_*.md` | Budget, pricing, model fallback |
| Plan 13 | `plan/13_*.md` | Context management, compaction, trajectory persistence |
| Plan 14 | `plan/14_*.md` | Observabilidade, EventBus, LiveTree |
