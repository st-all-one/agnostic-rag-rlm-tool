# TODO — arags-memory

> **OBSOLETO (pós plano 019):** este TODO descreve a arquitetura pré-refator.
> `SessionManager` (sessões multi-turno) e `TrajectoryEngine` (trajetórias) **foram
> removidos** do crate. O `arags-memory` agora cobre project/knowledge/persist/
> transfer/consolidation/history/watch/decay. A manutenção (consolidate/decay) é
> disparada por cron ou RPC admin no servidor. Veja `plan/019-cli-consolidation.md`.

## Status Atual

`MemoryEngine` totalmente refatorado em sub-módulos type-driven, com testes extraídos
para `tests/`, logging estruturado (`tracing`) e `ScopedTimer` nas hot paths, e
`cargo clippy`/`cargo test` limpos para o crate. Integração com `arags-core` via
`MemoryProvider` e sistema de decay implementados.

---

## Gaps Importantes (P1)

### 1. MemoryEngine integrado com core — ✅ CONCLUÍDO
- **Arquivo:** `src/engine/memory_api.rs:237`
- **Correção:** `impl MemoryProvider for MemoryEngine` implementado com a assinatura
  exata de `arags-core/src/memory.rs`:
  - `fn context(&self, task: &str) -> Result<Vec<String>, String>`
  - `fn save_trajectory(&self, input: &StartRunInput, result: &RlmRunResult) -> Result<(), String>`
- `arags-core` adicionado como dependência em `Cargo.toml`; `MemoryProvider` re-exportado em `lib.rs`.

### 2. Decay system — ✅ CONCLUÍDO
- **Arquivo:** `src/decay.rs` (self-contained, **sem** dependência de `arags-search`).
- **Correção:** `DecayConfig`, `SalienceInput`, `compute_salience` (recency + frequency +
  age, normalizado `[0,1]`), `should_evict`, `recency_score`, `frequency_score`,
  `age_penalty`, `now_ms`, `clamp`. Testes em `tests/decay_test.rs` (15 testes).

### 3. Context injection — ✅ CONCLUÍDO
- **Arquivo:** `src/engine/memory_api.rs:242`
- `MemoryEngine::context()` executa busca BM25 (via `search()`) e retorna os conteúdos
  de chunks como contexto. O solver em `arags-core` pode chamar `memory.context(task)`.

### 4. Trajectory persistence automática — ✅ CONCLUÍDO
- **Arquivo:** `src/engine/memory_api.rs:256`
- `save_trajectory()` converte a árvore de decisão (`RlmNode`) em `DecompositionNode`
  (`decompose_from_node`) e persiste via `store_trajectory`.

---

## Gaps Menores (P2)

### 5. TransferEngine testado (inter-projetos) — ✅ CONCLUÍDO
- **Arquivo:** `tests/transfer_integration_test.rs`
- Testes: transferência entre projetos via `MemoryEngine`, filtro de linguagem
  (`rust` vs `markdown`), e falha para source inexistente. 3 testes.

### 6. WatchMonitor não integrado ao CLI — ✅ MIGRADO
- O módulo legado (`src/watch.rs`) foi **removido**; o watcher de
  auto-atualização agora vive no client: `crates/arags-cli/src/watcher.rs`
  (`arags index --register`, daemon detached com debounce de 1 min).

### 7. Sem consolidação automática — ⚠️ ENGINE PRONTO / CLI FORA DE ESCOPO
- **Arquivo:** `src/consolidation.rs` (`ConsolidationEngine::consolidate`).
- **Verificação:** API existe (`ConsolidateOptions`, `ConsolidateResult`,
  deduplicação + remoção de padrões de baixa confiança).
- **Agendamento/CLI:** chamar em `arags consolidate` ou periodicamente — **follow-up**.

### 8. HistoryManager não integrado ao CLI — ⚠️ ENGINE PRONTO / CLI FORA DE ESCOPO
- **Arquivo:** `src/history.rs` (`HistoryManager::record/recent/count`).
- **Verificação:** API existe e é chamável.
- **CLI wiring:** `arags history` (modo CLI e servidor) — **follow-up**.

---

## Resumo de Verificação

| Gap | Status | Onde |
|-----|--------|------|
| #1 MemoryProvider | ✅ | `engine/memory_api.rs` |
| #2 decay.rs | ✅ | `decay.rs` + `decay_test.rs` |
| #3 context injection | ✅ | `engine/memory_api.rs` |
| #4 save_trajectory | ✅ | `engine/memory_api.rs` |
| #5 transfer integration | ✅ | `tests/transfer_integration_test.rs` |
| #6 watch CLI | ⚠️ engine pronto | `watch.rs` / CLI follow-up |
| #7 consolidation CLI | ⚠️ engine pronto | `consolidation.rs` / CLI follow-up |
| #8 history CLI | ⚠️ engine pronto | `history.rs` / CLI follow-up |

## Referências

| Plano | Arquivo | Descrição |
|-------|---------|-----------|
| Plan 04 | `plan/04_*.md` | Memory engine completa, consolidação, transfer, watch |
| Plan 05 | `plan/05_*.md` | Context injection no solver |
| Plan 13 | `plan/13_*.md` | Trajectory persistence |
| Plan 16 | `plan/16_*.md` | Decay system, salience |
