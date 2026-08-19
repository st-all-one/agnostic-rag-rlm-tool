# arlm-core

Engine RLM recursivo — o coração do sistema arlm.

## Responsabilidades

- **Engine**: Loop recursivo RLM (planner → solver → synthesizer)
- **Planner**: Decisão solve/decompose via LLM
- **Solver**: Resolução direta de tarefas
- **Synthesizer**: Merge de resultados dos filhos
- **Guardrails**: Detecção de ciclos, max depth/branching
- **Concurrency**: map_concurrent com buffer_unordered
- **Budget**: Controle de custo (USD/tokens/errors/time)
- **Events**: EventBus com broadcast channel
- **Cache**: ResultCache para dedup de subtasks

## Estrutura

```
src/
├── lib.rs          # Re-exports
├── logging.rs      # ScopedTimer, init_logging, log_metric
├── types.rs        # StartRunInput, RlmRunResult, RlmNode
├── engine.rs       # run_rlm_engine(), EngineState
├── planner.rs      # plan_node(), parse_planner_decision
├── solver.rs       # solve_task()
├── synthesizer.rs  # synthesize(), build_children_block
├── node.rs         # RlmNode tree structure
├── guardrails.rs   # detect_cycle, normalize_task
├── concurrency.rs  # map_concurrent
├── budget.rs       # RunBudget (atomic counters)
├── events.rs       # EventBus, RlmEvent
└── cache.rs        # ResultCache (HashMap + TTL)
```

## Uso

```rust
use arlm_core::{run_rlm_engine, StartRunInput};

let result = run_rlm_engine(StartRunInput {
    run_id: "abc123".to_string(),
    task: "Analise a arquitetura deste projeto".to_string(),
    backend: "openai".to_string(),
    model: Some("gpt-4".to_string()),
    project: "meu-projeto".to_string(),
    max_depth: 3,
    max_nodes: 20,
    concurrency: 4,
    max_budget: 1.0, // $1 USD
    ..Default::default()
}, llm_backend, memory, None, None).await?;

println!("Resultado: {}", result.final_output);
println!("Nós visitados: {}", result.stats.nodes_visited);
```

## Algoritmo RLM

```
task → planner → solve → solver → result
                  ↓
            decompose → subtasks
                           ↓
                    ┌──────┼──────┐
                 runNode runNode runNode
                    └──────┼──────┘
                           ↓
                       synthesizer
                           ↓
                         result
```

## Guardrails

- **Ciclo detection**: Normaliza tasks e compara com lineage
- **Max depth**: Força solve quando atinge profundidade máxima
- **Max nodes**: Limita número total de nós
- **Budget**: Para quando custo/tokens/tempo estouram
- **Error threshold**: Para após N erros consecutivos

## Budget

```rust
// Controle de custo com CAS loop para f64 correto
pub struct CostBudget {
    spent_bits: AtomicU64,  // f64 bits via CAS loop
    max: f64,
}
```

**Nota**: O `CostBudget` usa `compare_exchange_weak` para adição atômica correta de `f64`. Usar `fetch_add` em bits de `f64` não resulta em soma correta.

## Concorrência

```rust
// fan-out com limite real
let children = stream::iter(subtasks)
    .map(|task| run_node(task))
    .buffer_unordered(concurrency) // limite de tasks simultâneas
    .collect::<Vec<_>>()
    .await;
```

## Testes

```bash
cargo test -p arlm-core
```

78 testes cobrindo: engine, planner, solver, synthesizer, guardrails, concurrency, budget, events, cache.
