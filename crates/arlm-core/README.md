# arlm-core

Engine RLM recursivo — o coração do sistema `arlm` (Agnostic RLM).

## O que faz

Implementa o loop recursivo `planner → solver → synthesizer` que decompõe uma tarefa em
uma árvore de nós, resolve cada nó (opcionalmente via um loop REPL de código-execução-feedback)
e sintetiza os resultados dos filhos de volta para a raiz. O crate é **agent-agnostic**: define
traits desacoplados (`CodeSearch`, `MemoryProvider`) para que backends concretos de busca/memória
(vividos em outros crates) sejam injetados sem dependência rígida.

## Estrutura

```
src/
├── lib.rs                 # API pública: pub mod / pub use
├── types/
│   ├── mod.rs             # re-exports de enums/node/input
│   ├── enums.rs           # Action, NodeStatus, RlmBackend, CustomTool, CompactionPolicy
│   ├── input.rs           # StartRunInput (inclui CompactionPolicy)
│   └── node.rs            # RlmNode, RlmRunResult, RunStats
├── engine/
│   ├── mod.rs             # run_rlm_engine(_with_events): entrada; EventSink; save_trajectory (#3)
│   ├── node.rs            # run_node_owned: planner/solve/synthesize, recursão, guardrails
│   ├── state.rs           # EngineState: contadores atômicos lock-free
│   └── compactor.rs       # RootCompactor: fallback + summarize_with_llm (#6)
├── tools.rs               # ToolRegistry, ExecutableTool, ferramentas built-in, CodeSearch (#1)
├── memory.rs              # MemoryProvider trait + SharedMemory (#2, #3)
├── planner.rs             # plan_node / parse_planner_decision
├── solver.rs              # solve_task / solve_task_repl / PersistentSolver (#2)
├── synthesizer.rs         # synthesize / build_children_block + compactação por tokens (#4, #5)
├── router.rs              # DepthRouter: seleção de modelo por profundidade
├── budget.rs              # RunBudget: custo/tokens/erros/tempo (CAS loop p/ f64)
├── cache.rs               # ResultCache: TTL + LRU + invalidação por dependência (#10)
├── events.rs              # RlmEvent, EventBus (broadcast), EventSink (#7)
├── sampling.rs            # SamplingArgs com seed (#8)
├── token_counter.rs       # TokenCounter, get_context_limit, estimate (#9)
├── compaction.rs          # tipo Compaction (sumário de resultado de busca)
├── concurrency.rs         # map_concurrent: fan-out paralelo limitado
├── docker.rs              # DockerExecutor: execução sandboxed
├── repl.rs                # CodeExecutor, LlmQueryServer, find/format code blocks
├── guardrails.rs          # detecção de ciclo, normalização, sanitização
├── logging.rs             # ScopedTimer / Timer: timing estruturado
└── jsonl_logger.rs        # writer JSONL append-only (observabilidade)

tests/                     # 20 arquivos de teste de integração (em tests/, 196 testes)
benches/                   # rlm_loop.rs, search.rs (criterion)
```

## Funcionalidades (gaps do TODO concluídos)

- **#1 Busca real** — `SearchCodeTool` usa o trait `CodeSearch`; quando nenhum backend é
  injetado (`None`), retorna mensagem honesta `"search_code not configured: ..."` (sem fake).
- **#2 Injeção de memória** — `solve_task`/`solve_task_repl` aceitam `Option<Arc<dyn MemoryProvider>>`
  e prependam `context(task)` ao prompt do LLM.
- **#3 Persistência de trajectory** — `run_rlm_engine_with_events` chama `save_trajectory` ao final.
- **#4/#5 Compaction por tokens** — `compact_children_if_needed` compacta os filhos mais antigos
  via LLM quando excedem ~85% do contexto, respeitando `CompactionPolicy`.
- **#6 RootCompactor LLM** — `summarize_with_llm` resume saídas acumuladas.
- **#7 EventSink** — wrapper thread-safe sobre `Arc<EventBus>`.
- **#8 SamplingArgs.seed** — campo `seed: Option<u64>` propagado.
- **#9 Token counter** — heurística `estimate` (chars + pontuação) em vez de split por espaço.
- **#10 Cache** — invalidação por hash de dependência (`get_dep`/`put_dep`/`invalidate_dep`).

## Uso

```rust
use arlm_core::{run_rlm_engine, StartRunInput};

# async fn run(llm: std::sync::Arc<dyn arlm_llm::LlmBackend + Send + Sync>) -> anyhow::Result<()> {
let input = StartRunInput {
    run_id: std::sync::Arc::from("run-1"),
    task: "implementar feature X".to_string(),
    backend: arlm_core::RlmBackend::Ollama,
    ..Default::default()
};
let result = run_rlm_engine(input, llm).await?;
println!("{}", result.final_output);
# Ok(())
# }
```

## Testes

```bash
cargo test -p arlm-core        # 196 testes
cargo test --test engine_tests -p arlm-core
```

## Convenções

- Sem `unwrap`/`expect`/`panic` em `src/` (deny-lints do workspace); use `anyhow::Result` + `?`.
- Sem `unsafe` (forbid).
- Traits desacoplados (`CodeSearch`, `MemoryProvider`) injetados como `Arc<dyn Trait>`;
  default honesto (`None`) em vez de placeholder.
- Thread-safety: atômicos para contadores, `Arc<str>` para IDs, `EventSink` sobre `Arc<EventBus>`.
- Hot paths usam `ScopedTimer` + `tracing` com campos tipados.
- Testes vivem em `tests/` como integração; arquivos de teste podem usar `#![allow(...)]`.
