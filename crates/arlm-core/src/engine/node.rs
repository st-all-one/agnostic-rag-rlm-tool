use std::sync::Arc;

use parking_lot::Mutex;
use tracing::info;

use arlm_llm::LlmBackend;

use crate::budget::RunBudget;
use crate::cache::ResultCache;
use crate::engine::state::EngineState;
use crate::events::{EventBus, RlmEvent};
use crate::guardrails::{detect_cycle, normalize_task, sanitize_subtasks};
use crate::memory::MemoryProvider;
use crate::router::DepthRouter;
use crate::solver;
use crate::synthesizer;
use crate::token_counter::TokenCounter;
use crate::types::{AbortSignal, Action, NodeStatus, RlmNode, StartRunInput};

/// Parameters for a recursive node run. Uses owned types for Send safety.
pub struct RunNodeParamsOwned {
    pub task: String,
    pub depth: u32,
    pub lineage: Vec<String>,
    pub parent_id: Option<String>,
    pub input: StartRunInput,
    pub state: Arc<EngineState>,
    pub budget: Arc<RunBudget>,
    pub token_counter: Arc<TokenCounter>,
    pub events: Arc<EventBus>,
    pub cache: Arc<ResultCache>,
    pub llm: Arc<dyn LlmBackend + Send + Sync>,
    pub router: Arc<Mutex<DepthRouter>>,
    pub abort: Arc<AbortSignal>,
    pub memory: Option<Arc<dyn MemoryProvider>>,
}

/// Box the `run_node` future to make `Send` explicit.
fn run_node_boxed(
    params: RunNodeParamsOwned,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<RlmNode>> + Send>> {
    Box::pin(run_node_owned(params))
}

/// Recursively run a single node (owned params version).
///
/// Emits a `NodeEnd` event reflecting the node's terminal status.
pub(crate) async fn run_node_owned(params: RunNodeParamsOwned) -> anyhow::Result<RlmNode> {
    let node = run_node_inner(&params).await?;

    params.events.emit(RlmEvent::NodeEnd {
        run_id: params.input.run_id.clone(),
        node_id: node.id.clone(),
        status: node.status,
        duration_ms: node
            .finished_at_ms
            .map_or(0, |f| f.saturating_sub(node.started_at_ms)),
        cost: node.usage.cost_usd,
    });

    Ok(node)
}

/// Recursively run a single node given its parameters.
#[allow(clippy::too_many_lines)]
async fn run_node_inner(params: &RunNodeParamsOwned) -> anyhow::Result<RlmNode> {
    let node_id = params.state.next_node_id();
    params.state.record_visit(params.depth);

    // 0. GUARD: abort check
    if params.abort.is_cancelled() {
        return Ok(RlmNode::cancelled(&node_id, params.depth, &params.task));
    }

    // 1. GUARD: node budget check
    if params.state.nodes_visited() >= params.input.max_nodes {
        return Ok(RlmNode::skipped(&node_id, params.depth, &params.task));
    }

    // 1b. GUARD: financial/operational budget check
    if let Err(e) = params.budget.check() {
        return Ok(RlmNode::failed(
            &node_id,
            params.depth,
            &params.task,
            e.to_string(),
        ));
    }

    // 1c. GUARD: token budget check
    if let Err(e) = params.token_counter.check_budget() {
        return Ok(RlmNode::failed(
            &node_id,
            params.depth,
            &params.task,
            e.to_string(),
        ));
    }

    params.events.emit(RlmEvent::NodeStart {
        run_id: params.input.run_id.clone(),
        node_id: node_id.clone(),
        depth: params.depth,
        task: params.task.clone(),
        parent_id: params.parent_id.clone(),
    });

    // 2. CHECK forced solve
    if let Some(reason) = get_forced_solve_reason_owned(params) {
        let model = params
            .router
            .lock()
            .select_model(params.depth, params.input.model.as_deref());
        return solve_node_owned(&node_id, params, Some(&reason), Some(&model)).await;
    }

    // 3. ROUTE: suggest depth based on query complexity
    let suggested_depth = params.router.lock().suggest_depth(&params.task);
    let selected_model = params
        .router
        .lock()
        .select_model(params.depth, params.input.model.as_deref());
    info!(
        task = %params.task,
        current_depth = params.depth,
        suggested_depth,
        selected_model = %selected_model,
        "depth router suggestion"
    );

    // 4. PLAN
    let decision = crate::planner::plan_node(
        &params.task,
        params.depth,
        &params.input,
        params.llm.clone(),
        &params.budget,
        params.state.nodes_visited(),
        Some(&selected_model),
    )
    .await?;

    params.events.emit(RlmEvent::NodePlan {
        run_id: params.input.run_id.clone(),
        node_id: node_id.clone(),
        action: decision.action.to_string(),
        reason: decision.reason.clone(),
        subtasks: decision.subtasks.clone().unwrap_or_default(),
    });

    // 5. HANDLE PLAN DECISION
    match decision.action {
        Action::Solve => {
            let result = solve_node_owned(&node_id, params, None, Some(&selected_model)).await;
            let success = result.is_ok();
            params.router.lock().record_outcome(params.depth, success);
            result
        }
        Action::Decompose => {
            let subtasks = decision.subtasks.clone().unwrap_or_default();
            let subtasks = sanitize_subtasks(&subtasks, &params.task);

            let remaining_nodes = params.input.max_nodes - params.state.nodes_visited();
            let cost_per_subtask = params.input.max_budget / f64::from(params.input.max_nodes);

            let remaining_budget_usd = params.budget.summary().budget_remaining;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let max_children_count = (remaining_budget_usd / cost_per_subtask.max(0.001)) as usize;
            let max_children = std::cmp::min(
                std::cmp::min(
                    params.input.max_branching as usize,
                    remaining_nodes.saturating_sub(1) as usize,
                ),
                max_children_count,
            );
            let subtasks: Vec<String> = subtasks.into_iter().take(max_children).collect();

            if subtasks.len() < 2 {
                return solve_node_owned(&node_id, params, None, Some(&selected_model)).await;
            }

            // RECURSE: spawn each child as a separate task for true concurrency
            let concurrency = params.input.concurrency;
            let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));

            let mut handles = Vec::with_capacity(subtasks.len());
            for subtask in subtasks {
                let mut child_lineage = params.lineage.clone();
                child_lineage.push(normalize_task(&params.task));

                let owned = RunNodeParamsOwned {
                    task: subtask,
                    depth: params.depth + 1,
                    lineage: child_lineage,
                    parent_id: Some(node_id.clone()),
                    input: params.input.clone(),
                    state: params.state.clone(),
                    budget: params.budget.clone(),
                    token_counter: params.token_counter.clone(),
                    events: params.events.clone(),
                    cache: params.cache.clone(),
                    llm: params.llm.clone(),
                    router: params.router.clone(),
                    abort: params.abort.clone(),
                    memory: params.memory.clone(),
                };

                let sem = semaphore.clone();
                handles.push(tokio::spawn(async move {
                    let _permit = sem.acquire().await?;
                    run_node_boxed(owned).await
                }));
            }

            let mut children = Vec::with_capacity(handles.len());
            for handle in handles {
                match handle.await {
                    Ok(result) => children.push(result?),
                    Err(e) => {
                        tracing::error!(error = %e, "child task panicked");
                        return Err(anyhow::anyhow!("child task panicked: {e}"));
                    }
                }
            }

            let all_failed = children.iter().all(|c| c.status == NodeStatus::Failed);
            let all_cancelled = children.iter().all(|c| c.status == NodeStatus::Cancelled);

            if all_cancelled {
                return Ok(RlmNode::cancelled(&node_id, params.depth, &params.task)
                    .with_children(children));
            }
            if all_failed {
                return Ok(RlmNode::failed(
                    &node_id,
                    params.depth,
                    &params.task,
                    "all children failed".to_string(),
                )
                .with_children(children));
            }

            let result = synthesizer::synthesize(
                &params.task,
                &children,
                &params.input,
                params.llm.clone(),
                &params.budget,
                Some(&selected_model),
            )
            .await?;

            Ok(
                RlmNode::completed(&node_id, params.depth, &params.task, result)
                    .with_decision(decision)
                    .with_children(children),
            )
        }
    }
}

/// Solve a single node directly.
async fn solve_node_owned(
    node_id: &str,
    params: &RunNodeParamsOwned,
    forced_reason: Option<&str>,
    model_override: Option<&str>,
) -> anyhow::Result<RlmNode> {
    let model = model_override
        .or(params.input.model.as_deref())
        .unwrap_or("gpt-4o");

    params.events.emit(RlmEvent::NodeSolve {
        run_id: params.input.run_id.clone(),
        node_id: node_id.to_string(),
        model: model.to_string(),
        forced_reason: forced_reason.map(String::from),
    });

    let result = if params.input.mode == crate::types::RlmMode::Repl {
        solver::solve_task_repl(
            &params.task,
            &params.input,
            params.llm.clone(),
            &params.budget,
            &params.cache,
            params.memory.clone(),
            Some(model),
        )
        .await?
    } else {
        solver::solve_task(
            &params.task,
            &params.input,
            params.llm.clone(),
            &params.budget,
            &params.cache,
            params.memory.clone(),
            forced_reason,
            Some(model),
        )
        .await?
    };

    Ok(RlmNode::completed(
        node_id,
        params.depth,
        &params.task,
        result,
    ))
}

/// Determine if a node should be forced to solve (no decomposition).
#[must_use]
pub fn get_forced_solve_reason_owned(params: &RunNodeParamsOwned) -> Option<String> {
    if params.depth >= params.input.max_depth {
        return Some(format!("max depth {} reached", params.input.max_depth));
    }
    if params.state.nodes_visited() >= params.input.max_nodes {
        return Some(format!("max nodes {} reached", params.input.max_nodes));
    }
    let remaining = params.input.max_nodes - params.state.nodes_visited();
    if remaining < 2 {
        return Some(format!("budget exhausted ({remaining} remaining)"));
    }
    if detect_cycle(&params.task, &params.lineage) {
        return Some("cycle detected".to_string());
    }
    if params.budget.summary().budget_remaining <= 0.0 {
        return Some("budget in USD exhausted".to_string());
    }
    if params.budget.summary().errors_remaining == 0 {
        return Some("error threshold reached".to_string());
    }
    if params.budget.summary().time_remaining_ms == 0 {
        return Some("timeout reached".to_string());
    }
    None
}
