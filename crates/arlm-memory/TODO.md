# TODO — arlm-memory

> Memória multi-projetos: projetos, knowledge base, sessões, trajectories, persist.

## Status Atual

MemoryEngine com todos os sub-módulos. Falta integração com core (context injection, trajectory persistence) e decay system.

---

## Gaps Importantes (P1)

### 1. MemoryEngine não integrado com core
- **Arquivo:** `src/lib.rs` + `arlm-core/src/engine.rs`
- **Problema:** Engine RLM não recebe `MemoryEngine`, não busca contexto nem persiste trajectories.
- **Plano:** Plan 05 — Solver deve chamar `memory.context()` antes de cada LLM call.
- **Plano:** Plan 13 — Após cada run, persistir trajectory via `memory.save_trajectory()`.
- **Correção necessária:** Adicionar `MemoryEngine` como dependência do engine e propagar.

### 2. Sem decay system
- **Arquivo:** `src/` (não existe `decay.rs`)
- **Problema:** Não há módulo de decay/salience no arlm-memory. Existe em `arlm-search/decay.rs` mas não é integrado.
- **Plano:** Plan 16 — Decay system deve calcular salience de chunks baseado em access time, frequency, recency.
- **Correção necessária:** Mover `decay.rs` para arlm-memory OU integrar `arlm-search::decay` no memory engine.

### 3. Sem context injection
- **Arquivo:** `src/lib.rs` (método `context()`)
- **Problema:** `MemoryEngine::context()` pode existir mas não é chamado pelo solver.
- **Plano:** Plan 05 — Context injection deve buscar chunks relevantes da memória e injetar no prompt.
- **Correção necessária:** Solver deve chamar `memory.context(task, project)` e usar resultado.

### 4. Sem trajectory persistence automática
- **Arquivo:** `src/trajectory.rs` + `arlm-core/src/engine.rs`
- **Problema:** `TrajectoryEngine` existe mas não é chamado automaticamente após runs.
- **Plano:** Plan 13 — Após cada run, trajectory deve ser salva.
- **Correção necessária:** Engine deve chamar `memory.save_trajectory(input, result)` ao final.

---

## Gaps Menores (P2)

### 5. TransferEngine não testado
- **Arquivo:** `src/transfer.rs`
- **Problema:** `TransferEngine` existe mas não há testes de integração.
- **Plano:** Plan 04 — Transfer deve funcionar entre projetos.
- **Verificação necessária:** Testar transferência de knowledge entre projetos.

### 6. WatchMonitor não integrado ao CLI
- **Arquivo:** `src/watch.rs`
- **Problema:** `WatchMonitor` existe mas não é chamado pelo `arlm index --watch`.
- **Plano:** Plan 04 — Watch deve monitorar mudanças e re-indexar automaticamente.
- **Correção necessária:** Integrar `WatchMonitor` com comando `index --watch`.

### 7. Sem consolidação automática
- **Arquivo:** `src/consolidation.rs`
- **Problema:** `ConsolidationEngine` existe mas não roda automaticamente.
- **Plano:** Plan 04 — Consolidação deve rodar periodicamente ou sob demanda.
- **Correção necessária:** Agendar consolidação periódica ou chamar no `arlm consolidate`.

### 8. HistoryManager não integrado ao CLI
- **Arquivo:** `src/history.rs`
- **Problema:** `HistoryManager` existe mas `arlm history` pode não funcionar em modo servidor.
- **Plano:** Plan 04 — Histórico deve ser consultável.
- **Verificação necessária:** Confirmar que `arlm history` funciona em ambos os modos.

---

## Referências

| Plano | Arquivo | Descrição |
|-------|---------|-----------|
| Plan 04 | `plan/04_*.md` | Memory engine completa, consolidação, transfer, watch |
| Plan 05 | `plan/05_*.md` | Context injection no solver |
| Plan 13 | `plan/13_*.md` | Trajectory persistence |
| Plan 16 | `plan/16_*.md` | Decay system, salience |
