# arlm-core

## O que faz
Engine RLM recursivo (planner → solver → synthesizer) que decompõe tarefas em uma árvore de
nós e sintetiza os resultados. Define traits desacoplados (`CodeSearch`, `MemoryProvider`) para
injetar backends de busca/memória sem acoplar outros crates.

## Estrutura
- `src/lib.rs` — API pública (pub mod / pub use).
- `src/types/{mod,enums,node,input}.rs` — tipos de domínio (`RlmNode`, `StartRunInput`, `CompactionPolicy`, `RlmBackend`, `Action`, `NodeStatus`).
- `src/engine/mod.rs` — `run_rlm_engine(_with_events)`: entrada do loop, `EventSink`, `save_trajectory` (#3).
- `src/engine/node.rs` — `run_node_owned`: planner/solve/synthesize, recursão, guardrails.
- `src/engine/state.rs` — `EngineState`: contadores atômicos lock-free; gatilho de root-compaction.
- `src/engine/compactor.rs` — `RootCompactor`: fallback + `summarize_with_llm` (#6).
- `src/tools.rs` — `ToolRegistry`, `ExecutableTool`, ferramentas built-in, trait `CodeSearch` (#1).
- `src/memory.rs` — trait `MemoryProvider` + `SharedMemory` (#2, #3).
- `src/planner.rs` — `plan_node` / `parse_planner_decision` (planner via LLM).
- `src/solver.rs` — `solve_task` / `solve_task_repl` / `PersistentSolver` / `StateInspector` (#2).
- `src/synthesizer.rs` — `synthesize` / `build_children_block` + `compact_children_if_needed` (#4, #5).
- `src/router.rs` — `DepthRouter`: seleção de modelo por profundidade.
- `src/budget.rs` — `RunBudget`: custo/tokens/erros/tempo (CAS loop para f64).
- `src/cache.rs` — `ResultCache`: TTL + LRU + invalidação por dependência (#10).
- `src/events.rs` — `RlmEvent`, `EventBus` (broadcast), `EventSink` (#7).
- `src/sampling.rs` — `SamplingArgs` com `seed: Option<u64>` (#8).
- `src/token_counter.rs` — `TokenCounter`, `get_context_limit`, `estimate` (#9).
- `src/compaction.rs` — tipo `Compaction` (sumário de busca).
- `src/concurrency.rs` — `map_concurrent`: fan-out paralelo limitado.
- `src/docker.rs` — `DockerExecutor`: execução sandboxed.
- `src/repl.rs` — `CodeExecutor`, `LlmQueryServer`, `find_code_blocks`, `format_repl_result`.
- `src/guardrails.rs` — detecção de ciclo, normalização, sanitização de subtarefas.
- `src/logging.rs` — `ScopedTimer` / `Timer`: timing estruturado.
- `src/jsonl_logger.rs` — writer JSONL append-only (observabilidade).
- `tests/` — 20 arquivos de teste de integração (um por módulo, 196 testes).
- `benches/` — `rlm_loop.rs`, `search.rs` (criterion).

## Dependências
- Internas: `arlm-llm` (abstração de backend LLM).
- Externas: `anyhow` / `thiserror` (erros, sem unwrap/expect em src), `tokio` + `futures`
  (async + concorrência limitada), `parking_lot` (Mutex/RwLock p/ cache/router), `serde` /
  `serde_json` (serialização), `tracing` / `tracing-subscriber` (logs estruturados + timing),
  `sha2` / `hex` (chaves de cache / hash de dependência), `uuid` / `chrono` (IDs/timestamps),
  `async-trait` (traits assíncronos).

## Convenções deste módulo
- Sem `unwrap`/`expect`/`panic` em `src/` (deny-lints do workspace); use `anyhow::Result` + `?`.
- Sem `unsafe` (forbid).
- Traits desacoplados: `CodeSearch` e `MemoryProvider` são definidos aqui; impls concretas
  vivem em outros crates e são injetadas como `Arc<dyn Trait>` (comportamento honesto quando `None`).
- Thread-safety: atômicos (`AtomicU32`/`AtomicU64`) para contadores; `Arc<str>` para IDs;
  `EventSink` encapsula `Arc<EventBus>`.
- Observabilidade: hot paths (`solve_task`, `synthesize`, run de nó, compaction, cache) usam
  `ScopedTimer` e `tracing` com campos tipados.
- Testes vivem em `tests/` como integração; arquivos de teste podem conter
  `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`.

## Comandos úteis
```bash
# Checagem rápida (12 threads)
cargo check -p arlm-core

# Lint (limpo para este crate; ignora avisos de arlm-llm)
cargo clippy -p arlm-core --all-targets

# Testes de integração
cargo test -p arlm-core
cargo test --test engine_tests -p arlm-core

# Benchmarks
cargo bench -p arlm-core

# Formatação
cargo fmt -p arlm-core -- --check
```

## Migrations
- N/A — este crate não possui schema de banco próprio; persistência de trajectory/memória é
  feita por `MemoryProvider` (impl externa, tipicamente `arlm-memory`/`arlm-storage`).

## Rules
- `CodeSearch` e `MemoryProvider` são injetados como `Option<Arc<dyn Trait>>`; quando `None`,
  o comportamento é honesto (`"search_code not configured"` / sem contexto), nunca placeholder falso.
- Compaction respeita `CompactionPolicy` (`enabled`, `max_child_tokens`); só compacta quando
  os filhos excedem ~85% do limite de contexto do modelo.
- `save_trajectory` só é chamado se um `MemoryProvider` estiver configurado.
- `RootCompactor::summarize_with_llm` usa o `LlmBackend` para resumir; mantém fallback sem LLM.
- `SamplingArgs.seed`, quando presente, é propagado para a chamada LLM para reprodutibilidade.
- Cache com `dep_key` invalida entradas automaticamente quando a dependência muda (hash).
