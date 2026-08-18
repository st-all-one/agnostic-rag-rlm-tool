# RLM Orchestrator — Engine Recursivo

> **⚠️ Modo LLM (opt-in via `--llm`):** O engine RLM recursivo é o único
> componente do arlm que requer LLM. Ele é ativado apenas com a flag `--llm`.
> Todas as outras operações (search, context, persist, decay) são
> determinísticas e não precisam de LLM.

## Visão Geral

O `arlm-core` é o engine RLM recursivo que implementa o padrão Planner → Solver → Synthesizer. Diferente do Python original que usa REPL, este é um **orquestrador puro** que delega execução ao agente host.

```
┌──────────────────────────────────────────────────────────────┐
│                    arlm-core                                  │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐    │
│  │                   run_rlm_engine()                    │    │
│  │                                                      │    │
│  │  task ──► planner ──┬── solve ──► solver ──► result  │    │
│  │                     │                                │    │
│  │                     └── decompose ──► subtasks       │    │
│  │                                        │             │    │
│  │                              ┌─────────┼─────────┐   │    │
│  │                              ▼         ▼         ▼   │    │
│  │                           runNode  runNode  runNode  │    │
│  │                              │         │         │   │    │
│  │                              └─────────┼─────────┘   │    │
│  │                                        ▼             │    │
│  │                                    synthesizer       │    │
│  │                                        │             │    │
│  │                                        ▼             │    │
│  │                                      result          │    │
│  └──────────────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────────┘
```

## Algoritmo Completo

### Função Principal

```rust
pub async fn run_rlm_engine(
    input: StartRunInput,
    llm: &dyn LlmBackend,
    memory: &MemoryEngine,
    signal: Option<AbortSignal>,
    progress: Option<ProgressFn>,
    // NOVOS parâmetros (planos 12, 13, 14):
    budget: Arc<RunBudget>,          // plan 12 — budget global da run
    router: Arc<DepthRouter>,        // plan 12 — modelo/sampling por depth
    events: EventSink,               // plan 14 — eventos observáveis
    cache: Arc<ResultCache>,         // plan 14 — dedup de subtasks
    compaction: CompactionPolicy,    // plan 13 — política de context
    counter: Arc<TokenCounter>,      // plan 13 — contagem de tokens
) -> Result<RlmRunResult> {
    let started_at = Instant::now();
    let state = Arc::new(EngineState::new(input.max_nodes));

    events.emit(RlmEvent::RunStart {
        run_id: input.run_id.clone(),
        task: input.task.clone(),
        backend: input.backend.clone(),
        mode: input.mode.clone(),
        max_depth: input.max_depth,
        max_nodes: input.max_nodes,
        max_budget: input.max_budget,
        started_at: now_ms(),
    });

    let root = run_node(RunNodeParams {
        task: &input.task,
        depth: 0,
        lineage: vec![],
        parent_id: None,
        input: &input,
        llm,
        memory,
        state: &state,
        signal: &signal,
        progress: &progress,
        budget: &budget,
        router: &router,
        events: &events,
        cache: &cache,
        compaction: &compaction,
        counter: &counter,
    }).await?;

    let final_output = root.result.unwrap_or_default();
    let duration_ms = started_at.elapsed().as_millis() as u64;

    Ok(RlmRunResult {
        run_id: input.run_id,
        backend: input.backend.clone(),
        final_output,
        root,
        stats: RunStats {
            nodes_visited: state.nodes_visited(),
            max_depth_seen: state.max_depth_seen(),
            duration_ms,
        },
    })
}
```

### run_node — A Recursão

```rust
async fn run_node(params: RunNodeParams) -> Result<RlmNode> {
    let node_id = params.state.next_node_id();
    let started_at = Instant::now();

    // 1. GUARD: budget check (nós + financeiro)
    if params.state.nodes_visited() >= params.input.max_nodes {
        return Ok(RlmNode::skipped(node_id, params.depth, params.task));
    }

    // 1b. GUARD: budget financeiro/operacional [plan 12]
    // RunBudget é compartilhado — se estourou em qualquer branch, para tudo
    if let Err(e) = params.budget.check() {
        return Ok(RlmNode::failed_with_error(node_id, params.depth, params.task, e.to_string()));
    }

    // Emite evento de início
    params.events.emit(RlmEvent::NodeStart {
        run_id: params.input.run_id.clone(),
        node_id: node_id.clone(),
        depth: params.depth,
        task: params.task.clone(),
        parent_id: params.parent_id.clone(),
    });

    // 2. CHECK abort signal (com partial answer)
    if params.signal.is_cancelled() {
        let partial = collect_partial_from_descendants(params.lineage_cache());
        return Ok(RlmNode::cancelled_with_partial(node_id, params.depth, params.task, partial));
    }

    // 3. CHECK forced solve
    if let Some(reason) = get_forced_solve_reason(&params) {
        return solve_node(node_id, params, Some(&reason)).await;
    }

    // 4. PLAN
    let plan = plan_node(node_id, &params).await?;

    // 4b. Emite decisão do planner
    params.events.emit(RlmEvent::NodePlan {
        run_id: params.input.run_id.clone(),
        node_id: node_id.clone(),
        action: match plan.action { Action::Solve => "solve", Action::Decompose => "decompose" }.into(),
        reason: plan.reason.clone(),
        subtasks: plan.subtasks.clone().unwrap_or_default(),
    });

    // 5. HANDLE PLAN DECISION
    match plan.action {
        Action::Solve => {
            solve_node(node_id, params, None).await
        }
        Action::Decompose => {
            let subtasks = plan.subtasks.unwrap_or_default();

            // 6. SANITIZE subtasks
            let subtasks = sanitize_subtasks(&subtasks, &params.task);

            // 7. BUDGET check (nós + custo restante)
            let remaining_nodes = params.input.max_nodes - params.state.nodes_visited();
            let remaining_cost = params.budget.cost.remaining();
            let cost_per_subtask = params.input.max_budget / params.input.max_nodes as f64;

            let max_children = std::cmp::min(
                std::cmp::min(params.input.max_branching, remaining_nodes.saturating_sub(1)),
                (remaining_cost / cost_per_subtask.max(0.001)) as usize,
            );
            let subtasks: Vec<_> = subtasks.into_iter().take(max_children).collect();

            // 8. FALLBACK if < 2 valid subtasks
            if subtasks.len() < 2 {
                return solve_node(node_id, params, None).await;
            }

            // 9. RECURSE (parallel)
            let children = map_concurrent(subtasks, params.input.concurrency, |subtask| {
                let mut child_lineage = params.lineage.clone();
                child_lineage.push(normalize_task(params.task));

                run_node(RunNodeParams {
                    task: subtask,
                    depth: params.depth + 1,
                    lineage: child_lineage,
                    parent_id: Some(node_id),
                    ..params
                })
            }).await?;

            // 10. CHECK children results
            let all_failed = children.iter().all(|c| c.status == NodeStatus::Failed);
            let all_cancelled = children.iter().all(|c| c.status == NodeStatus::Cancelled);

            if all_cancelled {
                return Ok(RlmNode::cancelled(node_id, params.depth, params.task)
                    .with_children(children));
            }
            if all_failed {
                return Ok(RlmNode::failed(node_id, params.depth, params.task)
                    .with_children(children));
            }

            // 11. SYNTHESIZE
            let result = synthesize_node(node_id, &params, &children).await?;

            Ok(RlmNode::completed(node_id, params.depth, params.task, result)
                .with_decision(plan)
                .with_children(children))
        }
    }
}
```

## Três Chamadas LLM

### 1. Planner

```rust
async fn plan_node(node_id: &str, params: &RunNodeParams) -> Result<PlannerDecision> {
    // Roteia modelo + sampling pelo depth [plan 12]
    let config = params.router.resolve(params.depth);

    let budget_summary = params.budget.summary();

    let prompt = format!(
        r#"You are a recursion controller. Analyze the task and decide whether to solve it directly or decompose it into subtasks.

Task: {task}

Context:
- Depth: {depth}/{max_depth}
- Nodes visited: {visited}/{max_nodes}
- Remaining budget: ${budget:.2} / {tokens} tokens / {errors} errors / {time}s

Return JSON: {{"action": "solve"|"decompose", "reason": "...", "subtasks": ["..."]}}

If the task is atomic or budget is low, choose "solve".
If the task can be meaningfully split, choose "decompose" with 2-5 subtasks.
Decomposition multiplies cost — only decompose when it clearly helps."#,
        task = params.task,
        depth = params.depth,
        max_depth = params.input.max_depth,
        visited = params.state.nodes_visited(),
        max_nodes = params.input.max_nodes,
        budget = budget_summary.budget_remaining,
        tokens = budget_summary.tokens_remaining,
        errors = budget_summary.errors_remaining,
        time = budget_summary.time_remaining_ms / 1000,
    );

    let response = complete_with_retry(
        params.llm,
        &CompletionRequest {
            prompt,
            system: Some("You are a recursion controller for an RLM system.".into()),
            model: Some(config.model.clone()),
            sampling: SamplingArgs::planner(),
            max_tokens: config.max_tokens,
            ..Default::default()
        },        &params.input.retry_policy,
        &params.budget,
    ).await?;

    // Rastreia custo e emite evento
    params.budget.record_call(response.usage.clone());
    params.events.emit(RlmEvent::CostUpdate {
        run_id: params.input.run_id.clone(),
        spent: params.budget.cost.spent(),
        budget: params.input.max_budget,
    });

    parse_planner_decision(&response.text)
}
```

### 2. Solver

```rust
async fn solve_node(
    node_id: &str,
    params: &RunNodeParams,
    forced_reason: Option<&str>,
) -> Result<RlmNode> {
    // Config por depth (modelo barato em folhas) [plan 12]
    let config = params.router.resolve(params.depth);

    // CACHE: tarefa idêntica já resolvida? [plan 14]
    if let Some(cached) = params.cache.get(&params.task, &params.input.project) {
        params.events.emit(RlmEvent::CacheHit {
            run_id: params.input.run_id.clone(),
            node_id: node_id.to_string(),
            task_hash: ResultCache::task_hash(&params.task),
        });
        return Ok(RlmNode::completed_with_cache(
            node_id, params.depth, params.task, cached,
        ));
    }

    params.events.emit(RlmEvent::NodeSolve {
        run_id: params.input.run_id.clone(),
        node_id: node_id.to_string(),
        model: config.model.clone(),
        forced_reason: forced_reason.map(String::from),
    });

    let prompt = if let Some(reason) = forced_reason {
        format!(
            r#"Solve this task directly. You were forced to solve because: {reason}

Task: {task}

Provide a concrete, actionable answer."#,
            task = params.task,
            reason = reason,
        )
    } else {
        format!(
            r#"Solve this task directly and return a concrete answer.

Task: {task}"#,
            task = params.task,
        )
    };

    // Busca contexto relevante na memória
    let context = params.memory.context(params.task, &params.input.project, OutputFormat::Prompt)?;

    let response = complete_with_retry(
        params.llm,
        &CompletionRequest {
            prompt: format!("{}\n\n{}", context, prompt),
            system: Some("You are a worker node in an RLM system. Solve the task directly.".into()),
            model: Some(config.model.clone()),
            sampling: SamplingArgs::solver(),
            max_tokens: config.max_tokens,
            ..Default::default()
        },
        &params.input.retry_policy,
        &params.budget,
    ).await?;

    // Rastreia custo e cacheia resultado
    params.budget.record_call(response.usage.clone());
    params.cache.put(&params.task, &params.input.project, &response.text);

    Ok(RlmNode::completed(node_id, params.depth, params.task, response.text))
}
```

### 3. Synthesizer

```rust
async fn synthesize_node(
    node_id: &str,
    params: &RunNodeParams,
    children: &[RlmNode],
) -> Result<String> {
    let config = params.router.resolve(params.depth);

    params.events.emit(RlmEvent::NodeSynthesize {
        run_id: params.input.run_id.clone(),
        node_id: node_id.to_string(),
        model: config.model.clone(),
        children_count: children.len(),
        compacted: false,
    });

    // COMPACTION: se outputs dos children excedem o contexto [plan 13]
    let children_block = build_children_block(
        children,
        &config.model,
        params.counter,
        params.compaction,
        params.llm,
    ).await?;

    let prompt = format!(
        r#"You are the synthesizer node. Merge the outputs of child nodes into one coherent answer.

Parent task: {parent_task}

Children outputs:
{children_block}

Synthesize a unified, complete answer. Handle failed/cancelled children gracefully."#,
        parent_task = params.task,
        children_block = children_block,
    );

    let response = complete_with_retry(
        params.llm,
        &CompletionRequest {
            prompt,
            system: Some("You are a synthesizer in an RLM system. Merge child outputs into one answer.".into()),
            model: Some(config.model.clone()),
            sampling: SamplingArgs::synthesizer(),
            max_tokens: config.max_tokens,
            ..Default::default()
        },
        &params.input.retry_policy,
        &params.budget,
    ).await?;

    params.budget.record_call(response.usage.clone());
    Ok(response.text)
}
```

## Guardrails

### Ciclo Detection

```rust
fn detect_cycle(task: &str, lineage: &[String]) -> bool {
    let normalized = normalize_task(task);
    lineage.iter().any(|l| l == &normalized)
}

fn normalize_task(task: &str) -> String {
    task.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
```

### Budget Management

```rust
fn get_forced_solve_reason(params: &RunNodeParams) -> Option<String> {
    if params.depth >= params.input.max_depth {
        return Some(format!("max depth {} reached", params.input.max_depth));
    }
    if params.state.nodes_visited() >= params.input.max_nodes {
        return Some(format!("max nodes {} reached", params.input.max_nodes));
    }
    let remaining = params.input.max_nodes - params.state.nodes_visited();
    if remaining < 2 {
        return Some(format!("budget exhausted ({} remaining)", remaining));
    }
    if detect_cycle(params.task, &params.lineage) {
        return Some("cycle detected".into());
    }
    // Budget financeiro restante [plan 12]
    if params.budget.cost.remaining() <= 0.0 {
        return Some("budget in USD exhausted".into());
    }
    if params.budget.errors.remaining() <= 0 {
        return Some("error threshold reached".into());
    }
    if params.budget.time.is_expired() {
        return Some("timeout reached".into());
    }
    None
}
```

### Subtask Sanitization

```rust
fn sanitize_subtasks(subtasks: &[String], parent_task: &str) -> Vec<String> {
    let parent_normalized = normalize_task(parent_task);

    subtasks.iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| normalize_task(s) != parent_normalized)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}
```

## Concorrência

### map_concurrent

`buffer_unordered(concurrency)` é o padrão do guia Rust para fan-out com
limite real: cria no máximo `concurrency` tasks simultâneas (o semáforo
ingênuo que lançava todas as tasks de uma vez ignorava o limite).

```rust
use futures::stream::{self, StreamExt};

async fn map_concurrent<T, F, Fut, R>(
    items: Vec<T>,
    concurrency: usize,
    f: F,
) -> Result<Vec<R>>
where
    T: Send + 'static,
    F: Fn(T) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<R>> + Send + 'static,
    R: Send + 'static,
{
    let results: Vec<R> = stream::iter(items)
        .map(|item| {
            // Semaphore ainda útil para BACKPRESSURE além do spawn
            let f = &f;
            async move { f(item).await }
        })
        .buffer_unordered(concurrency)   // limita tasks de verdade
        .collect::<Vec<_>>()
        .await;

    // Propaga o primeiro erro
    results.into_iter().collect()
}
```

> **Nota de CPU-bound:** se `f` fizer trabalho pesado (busca/DB sincrônica),
> envolva o corpo com `tokio::task::spawn_blocking` antes de chamar, para não
> ocupar os worker threads do runtime (ver seção "spawn_blocking" abaixo).

### EngineState — contadores atômicos (sem lock contention)

`nodes_visited()` / `max_depth_seen()` / `next_node_id()` são chamados em
**todo** guardrail de cada nó concorrente. Com `Mutex` isso viraria lock
contention no caminho quente do fan-out. Usar `AtomicU32`/`AtomicU64`
(guia Rust: atomics são mais leves que Mutex para tipos simples):

```rust
pub struct EngineState {
    nodes_visited: AtomicU32,
    max_depth_seen: AtomicU32,
    next_id: AtomicU64,
    max_nodes: u32,
}

impl EngineState {
    pub fn new(max_nodes: u32) -> Self {
        Self {
            nodes_visited: AtomicU32::new(0),
            max_depth_seen: AtomicU32::new(0),
            next_id: AtomicU64::new(1),
            max_nodes,
        }
    }

    pub fn next_node_id(&self) -> String {
        format!("n{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    pub fn nodes_visited(&self) -> u32 {
        self.nodes_visited.load(Ordering::Relaxed)
    }

    pub fn max_depth_seen(&self) -> u32 {
        self.max_depth_seen.load(Ordering::Relaxed)
    }

    pub fn record_visit(&self, depth: u32) -> u32 {
        let v = self.nodes_visited.fetch_add(1, Ordering::Relaxed) + 1;
        self.max_depth_seen.fetch_max(depth, Ordering::Relaxed);
        v
    }
}
```

- `Relaxed` é suficiente: são contadores de heurística (guardrails), não
  sincronização de dados.
- `AtomicU32::fetch_max` evita o `if` racy com `Mutex`.
- Chamar `record_visit(depth)` no início de `run_node` em vez de
  `nodes_visited()` para leituras + incremento atômicos.

### spawn_blocking (CPU/IO-bound fora do runtime async)

O engine é async (chamadas LLM), mas busca/DB são síncronos. Seguir o guia
Rust: **use `spawn_blocking` para CPU-bound** e evite `block_on` dentro de
async (deadlock).

```rust
use tokio::task;

// Em vez de chamar `memory.context(...)` (busca síncrona) direto no async fn:
let context = task::spawn_blocking({
    let memory = params.memory.clone();
    let task = params.task.clone();
    let project = params.input.project.clone();
    move || memory.context(&task, &project, OutputFormat::Prompt)
})
.await??;
```

O mesmo vale para `run_rlm_engine` quando chamado de fora do runtime — e para
o `serve` mode, onde múltiplas requests não podem bloquear os workers do Tokio.

## Tipos

```rust
pub struct StartRunInput {
    pub run_id: String,
    pub task: String,
    pub backend: RlmBackend,
    pub mode: RlmMode,
    pub model: Option<String>,
    pub project: String,
    pub tools_profile: ToolsProfile,
    pub max_depth: u32,
    pub max_nodes: u32,
    pub max_branching: u32,
    pub concurrency: usize,
    pub timeout_ms: u64,
    // NOVOS (planos 12-14):
    pub max_budget: f64,            // USD máximo por run
    pub max_tokens: u64,            // tokens máximos por run
    pub max_errors: u32,            // erros máximos antes de abortar
    pub agent: String,              // quem iniciou (accountability)
    pub retry_policy: RetryPolicy,  // retry + backoff
    pub enable_cache: bool,         // dedup de subtasks
    pub compaction: CompactionPolicy,
}

pub enum RlmMode {
    Auto,
    Solve,
    Decompose,
}

pub struct RlmNode {
    pub id: String,
    pub depth: u32,
    pub task: String,
    pub status: NodeStatus,
    pub decision: Option<PlannerDecision>,
    pub started_at: Instant,
    pub finished_at: Option<Instant>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub children: Vec<RlmNode>,
    // NOVOS (planos 12-13):
    pub usage: UsageSummary,               // custo/tokens deste nó + filhos
    pub partial_answer: Option<String>,    // resultado parcial se abortado
    pub cached: bool,                      // se veio do ResultCache
}

pub struct RlmRunResult {
    pub run_id: String,
    pub backend: String,
    pub final_output: String,
    pub root: RlmNode,
    pub stats: RunStats,
    // NOVOS (plano 12):
    pub usage: UsageSummary,               // uso agregado da run
}

pub struct PlannerDecision {
    pub action: Action,
    pub reason: String,
    pub subtasks: Option<Vec<String>>,
}

pub enum Action {
    Solve,
    Decompose,
}
```

### Tipagem otimizada de prompts

- `CompletionRequest.system` deve ser `Option<Cow<'static, str>>` (não `String`):
  system prompts são constantes (`&'static str`) e **não devem alocar** a cada
  chamada LLM. `Some("...".into())` sobre `&'static str` → `Cow::Borrowed`, zero alloc.
- `StartRunInput.run_id` / `RlmNode.id` / `RlmEvent.*_id` → `Arc<str>` (ou `u64`
  compacto) em vez de `String` clonada em cada `emit` do broadcast — o
  `tokio::sync::broadcast` clona o evento inteiro por subscriber; IDs compartilhados
  via `Arc<str>` tornam o clone barato (guia Rust: Arc para dados imutáveis
  compartilhados).

## Integração com Memória

O RLM orchestrator se integra com o `arlm-memory` de três formas:

1. **Context injection:** Antes de cada chamada LLM (planner/solver/synthesizer), o engine busca contexto relevante na memória e injeta no prompt.

2. **Result persistence:** Após cada run, o engine salva a árvore de decisão e resultados na memória, enriquecendo o knowledge base para futuras consultas.

3. **Trajectory + sessões:** Salva a trajectória completa (plan 13) e suporta sessões multi-turn para conversas persistentes.

```rust
// No solver:
let context = memory.context(&task, &project, OutputFormat::Prompt)?;
let prompt = format!("{}\n\n{}", context, solver_prompt);

// Após o run:
memory.save_run_result(&run_result)?;

// Trajectória completa para aprendizado [plan 13]:
let trajectory = RunTrajectory::from_run(&run_result);
memory.save_trajectory(&trajectory)?;
```
```
