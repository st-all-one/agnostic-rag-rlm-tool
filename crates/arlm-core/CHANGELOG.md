# Changelog

Todas as mudanças notáveis deste crate são documentadas neste arquivo.

O formato segue [Keep a Changelog](https://keepachangelog.com/pt-BR/1.0.0/),
e o versionamento [SemVer](https://semver.org/lang/pt-BR/).

## [Unreleased]

### Added
- **QA-Cache engine (plan 017):** `src/qa_cache.rs` com `QaThresholds`
  (configurável), `QaPlan` e `resolve_plan(similarity, jaccard, t)` — mapeia a
  similaridade de pergunta (cosseno) **e** a checagem secundária (Jaccard de
  provenance) em um plano de digestão com widening adaptativo; invariante
  `provenance_k ≤ digest_k ≤ novel_k` sempre respeitada. Módulo puro (sem
  storage/embedder), coberto por testes unitários.

### Adicionado
- Traits desacoplados `CodeSearch` (`tools.rs`) e `MemoryProvider` (`memory.rs`) para injeção
  de backends de busca/memória sem dependência rígida de outros crates (#1, #2, #3).
- `EventSink` (`events.rs`): wrapper thread-safe sobre `Arc<EventBus>` (#7).
- `RootCompactor::summarize_with_llm` para sumarização LLM das saídas acumuladas (#6).
- `compact_children_if_needed` no `synthesizer` com compaction por tokens respeitando
  `CompactionPolicy` (#4, #5).
- `SamplingArgs.seed: Option<u64>` propagado para as chamadas LLM (#8).
- `TokenCounter::estimate` com heurística chars+pontuação (substitui split por espaço) (#9).
- Invalidação por dependências no `ResultCache` (`get_dep`/`put_dep`/`invalidate_dep`) (#10).

### Alterado
- **Reorganização type-driven**: `types.rs` dividido em `types/{mod,enums,node,input}.rs`;
  `engine.rs` dividido em `engine/{mod,node,state,compactor}.rs`; ferramentas movidas para
  `tools.rs`; memória movida para `memory.rs`. Nenhum arquivo fonte excede ~300 linhas.
- Persistência de trajectory no fim de `run_rlm_engine_with_events` quando `MemoryProvider`
  está configurado (#3).
- Logs estruturados (`tracing` com campos tipados) e `ScopedTimer`/`Timer` em hot paths
  (solve, synthesize, run de nó, compaction, cache).
- Testes extraídos de `src/` para `tests/` (20 arquivos de integração, 196 testes).
- `README.md`, `MODULE.md` e `TODO.md` atualizados para a nova estrutura.

### Removido
- Placeholder fake do `SearchCodeTool` — agora retorna mensagem honesta `"search_code not
  configured: ..."` quando nenhum backend é injetado (#1).

## [0.1.0] - 2026-08-19

### Added
- Engine RLM recursivo com planner/solver/synthesizer
- Sistema de logging estruturado com ScopedTimer
- Profiling com timed! e timed_verbose! macros
- Flag --verbose para logs detalhados
- Guardrails: ciclo detection, max depth/branching, budget
- Concorrência com buffer_unordered
- Budget management (USD/tokens/errors/time)
- EventBus com broadcast channel
- ResultCache para dedup de subtasks
- EngineState com atomic counters
- Unit tests (78 testes)

### Fixed
- **Bug crítico em CostBudget**: `f64` não somava corretamente usando `fetch_add` em bits
  - Corrigido para usar CAS loop (compare_exchange_weak) para adição atômica correta de f64
