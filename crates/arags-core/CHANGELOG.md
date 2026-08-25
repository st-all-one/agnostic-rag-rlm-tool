# Changelog

Todas as mudanças notáveis deste crate são documentadas neste arquivo.

O formato segue [Keep a Changelog](https://keepachangelog.com/pt-BR/1.0.0/),
e o versionamento [SemVer](https://semver.org/lang/pt-BR/).

## [Unreleased]

### Added — plan 022: modelo puro de explorações
- **Novo `src/exploration.rs`**: `ExplorationPayload` (serde tolerante, fonte
  única client/server/storage), consts de domínio (`STATUS_*`, `ROLE_*`,
  `TEMPLATE_VERSION_V1`) e o **modelo de confiança**: `ConfidenceConfig` com
  defaults conservadores (precision > recall), `classify()` com limiares
  duplos (`hit_high`/`hit_low`) e `confidence_score()` — função pura
  `sim × drift × age + feedback` com floors de degradação e conversões exatas
  saturadas em 2^24. 7 testes unitários (`exploration/tests.rs`) +
  5 proptests de monotonicidade/boundedness
  (`tests/exploration_proptest.rs`; dev-dep `proptest` adicionada).

### Added (plan 021 — fonte única de verdade do domínio RLM)
- `src/rlm.rs` — constantes e payload compartilhados entre client (volunteer),
  server e storage, antes duplicados em 3 crates:
  - `DEFAULT_RLM_LEASE_MS` (500s);
  - escada de prioridades nomeada: `PRIORITY_CANCELLED`(0)/`RETRY`(1)/
    `CASCADE`(3)/`FRESH`(5)/`PARKED`(9);
  - `RlmJobPayload` (`Serialize + Deserialize`, todos os campos com
    `#[serde(default)]`) — refs de input (`chunk_ids`/`node_ids`/`hashes`/
    `texts`) + metadata de template; writers omitem vetores vazios.
  Re-exportado por `arags-storage::sqlite::rlm`.
- Testes: round-trip do payload (inclui tolerância a payloads parciais/legacy)
  e serialização compacta.

### Added
- `EMBEDDING_DIMS` (384): fonte única da dimensionalidade do modelo fixo
  all-MiniLM-L6-v2, usada por storage/server/embedding.

### Removed (limpeza pós-019/020)
- `src/types/{enums,input,node}.rs` (placeholders vazios do engine RLM) e o
  trait `MemoryProvider` (`src/memory.rs`) — sem uso em todo o workspace.
- Dependência morta `arags-llm` — o crate ficou 100% LLM-free no grafo.

### Added (auditoria plan 020)
- `qa_cache::chunk_content_hash(content)` — hash canônico SHA-256 hex do texto
  do chunk, movido do `arags-storage` para cá: cliente (digest-once
  `StoreAnswer.source_hashes`) e servidor (staleness) compartilham a mesma
  implementação sem o client depender de storage. Re-exportado pelo storage.

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
