use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use anyhow::Result;
use parking_lot::Mutex;
use tracing::info;

use arlm_llm::LlmBackend;

use crate::budget::RunBudget;
use crate::cache::ResultCache;
use crate::events::{EventBus, RlmEvent};
use crate::guardrails::{detect_cycle, normalize_task, sanitize_subtasks};
use crate::logging::ScopedTimer;
use crate::planner;
use crate::router::DepthRouter;
use crate::solver;
use crate::synthesizer;
use crate::token_counter::TokenCounter;
use crate::token_counter::get_context_limit;
use crate::types::{Action, NodeStatus, RlmNode, RlmRunResult, RunStats, StartRunInput, now_ms};
use crate::types::AbortSignal;

/// Root-level compaction threshold: trigger when context reaches 60% of model limit.
const ROOT_COMPACTION_THRESHOLD: f64 = 0.60;

/// Shared engine state with atomic counters.
#[derive(Debug)]
pub struct EngineState {
    nodes_visited: AtomicU32,
    max_depth_seen: AtomicU32,
    next_id: AtomicU64,
    total_output_tokens: AtomicU32,
}

impl EngineState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes_visited: AtomicU32::new(0),
            max_depth_seen: AtomicU32::new(0),
            next_id: AtomicU64::new(1),
            total_output_tokens: AtomicU32::new(0),
        }
    }

    #[must_use]
    pub fn next_node_id(&self) -> String {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        format!("n{id}")
    }

    pub fn record_visit(&self, depth: u32) {
        self.nodes_visited.fetch_add(1, Ordering::Relaxed);
        self.max_depth_seen.fetch_max(depth, Ordering::Relaxed);
    }

    /// Record output tokens from a node.
    pub fn record_output_tokens(&self, tokens: u32) {
        self.total_output_tokens.fetch_add(tokens, Ordering::Relaxed);
    }

    /// Check if root-level compaction is needed.
    pub fn needs_root_compaction(&self, model: &str) -> bool {
        let model_limit = get_context_limit(model);
        let threshold = (model_limit as f64 * ROOT_COMPACTION_THRESHOLD) as u32;
        let total = self.total_output_tokens.load(Ordering::Relaxed);
        total >= threshold
    }

    #[must_use]
    pub fn nodes_visited(&self) -> u32 {
        self.nodes_visited.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn max_depth_seen(&self) -> u32 {
        self.max_depth_seen.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn total_output_tokens(&self) -> u32 {
        self.total_output_tokens.load(Ordering::Relaxed)
    }
}

impl Default for EngineState {
    fn default() -> Self {
        Self::new()
    }
}

/// Root-level compactor that summarizes accumulated output when context gets too large.
pub struct RootCompactor {
    /// Accumulated output summaries.
    summaries: Vec<String>,
    /// Maximum summaries to keep.
    max_summaries: usize,
}

impl RootCompactor {
    /// Create a new root compactor.
    pub fn new() -> Self {
        Self {
            summaries: Vec::new(),
            max_summaries: 10,
        }
    }

    /// Add an output to the compactor.
    pub fn add_output(&mut self, output: &str) {
        // Truncate long outputs
        let truncated = if output.len() > 1000 {
            format!("{}...", &output[..1000])
        } else {
            output.to_string()
        };
        self.summaries.push(truncated);

        // Keep only the most recent summaries
        if self.summaries.len() > self.max_summaries {
            let drain = self.summaries.len() - self.max_summaries;
            self.summaries.drain(..drain);
        }
    }

    /// Get a summary of all accumulated outputs.
    pub fn get_summary(&self) -> String {
        if self.summaries.is_empty() {
            return "No outputs accumulated.".to_string();
        }

        format!(
            "Accumulated outputs ({}):\n{}",
            self.summaries.len(),
            self.summaries.join("\n---\n")
        )
    }

    /// Clear the summaries.
    pub fn clear(&mut self) {
        self.summaries.clear();
    }

    /// Get the number of accumulated outputs.
    pub fn len(&self) -> usize {
        self.summaries.len()
    }

    /// Check if the compactor is empty.
    pub fn is_empty(&self) -> bool {
        self.summaries.is_empty()
    }
}

impl Default for RootCompactor {
    fn default() -> Self {
        Self::new()
    }
}

/// Parameters for a recursive node run. Uses owned types for Send safety.
struct RunNodeParamsOwned {
    task: String,
    depth: u32,
    lineage: Vec<String>,
    parent_id: Option<String>,
    input: StartRunInput,
    state: Arc<EngineState>,
    budget: Arc<RunBudget>,
    token_counter: Arc<TokenCounter>,
    events: Arc<EventBus>,
    cache: Arc<ResultCache>,
    llm: Arc<dyn LlmBackend + Send + Sync>,
    router: Arc<Mutex<DepthRouter>>,
    abort: Arc<AbortSignal>,
}

/// Run the RLM engine on a task with an internal event bus.
///
/// # Errors
///
/// Returns an error if the engine encounters a fatal failure.
#[allow(clippy::cast_possible_truncation)]
pub async fn run_rlm_engine(
    input: StartRunInput,
    llm: Arc<dyn LlmBackend + Send + Sync>,
) -> Result<RlmRunResult> {
    run_rlm_engine_with_events(input, llm, EventBus::new()).await
}

/// Run the RLM engine on a task, broadcasting `RlmEvent`s on the given bus.
///
/// # Errors
///
/// Returns an error if the engine encounters a fatal failure.
#[allow(clippy::cast_possible_truncation)]
pub async fn run_rlm_engine_with_events(
    input: StartRunInput,
    llm: Arc<dyn LlmBackend + Send + Sync>,
    event_bus: EventBus,
) -> Result<RlmRunResult> {
    let _timer = ScopedTimer::new("run_rlm_engine");
    let started_at = Instant::now();
    let state = Arc::new(EngineState::new());
    let budget = Arc::new(RunBudget::new(
        input.max_budget,
        input.max_tokens,
        input.max_errors,
        input.timeout_ms,
    ));
    let token_counter = Arc::new(TokenCounter::new(input.max_tokens as u32));
    let events = Arc::new(event_bus);
    let cache = Arc::new(ResultCache::default_config());
    let router = Arc::new(Mutex::new(DepthRouter::new()));

    events.emit(RlmEvent::RunStart {
        run_id: input.run_id.clone(),
        task: input.task.clone(),
        backend: input.backend.to_string(),
        mode: format!("{:?}", input.mode).to_lowercase(),
        max_depth: input.max_depth,
        max_nodes: input.max_nodes,
        max_budget: input.max_budget,
        started_at_ms: now_ms(),
    });

    let root = run_node_owned(RunNodeParamsOwned {
        task: input.task.clone(),
        depth: 0,
        lineage: Vec::new(),
        parent_id: None,
        input: input.clone(),
        state: state.clone(),
        budget: budget.clone(),
        token_counter: token_counter.clone(),
        events: events.clone(),
        cache: cache.clone(),
        llm: llm.clone(),
        router: router.clone(),
        abort: Arc::new(input.abort.clone()),
    })
    .await?;

    let final_output = root.result.clone().unwrap_or_default();
    #[allow(clippy::cast_possible_truncation)]
    let duration_ms = started_at.elapsed().as_millis() as u64;
    let run_stats = RunStats {
        nodes_visited: state.nodes_visited(),
        max_depth_seen: state.max_depth_seen(),
        duration_ms,
    };

    events.emit(RlmEvent::RunEnd {
        run_id: input.run_id.clone(),
        duration_ms,
        nodes_visited: run_stats.nodes_visited,
    });

    info!(
        run_id = input.run_id.as_ref(),
        nodes = run_stats.nodes_visited,
        max_depth = run_stats.max_depth_seen,
        duration_ms = run_stats.duration_ms,
        "RLM run completed"
    );

    Ok(RlmRunResult {
        run_id: input.run_id.to_string(),
        backend: input.backend.to_string(),
        final_output,
        root,
        stats: run_stats,
    })
}

/// Box the `run_node` future to make Send explicit.
fn run_node_boxed(
    params: RunNodeParamsOwned,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<RlmNode>> + Send>> {
    Box::pin(run_node_owned(params))
}

/// Recursively run a single node (owned params version).
///
/// Emits a `NodeEnd` event reflecting the node's terminal status.
async fn run_node_owned(params: RunNodeParamsOwned) -> Result<RlmNode> {
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
async fn run_node_inner(params: &RunNodeParamsOwned) -> Result<RlmNode> {
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
        let model = params.router.lock().select_model(params.depth, params.input.model.as_deref());
        return solve_node_owned(&node_id, params, Some(&reason), Some(&model)).await;
    }

    // 3. ROUTE: suggest depth based on query complexity
    let suggested_depth = params.router.lock().suggest_depth(&params.task);
    let selected_model = params.router.lock().select_model(params.depth, params.input.model.as_deref());
    info!(
        task = %params.task,
        current_depth = params.depth,
        suggested_depth,
        selected_model = %selected_model,
        "depth router suggestion"
    );

    // 4. PLAN
    let decision = planner::plan_node(
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
) -> Result<RlmNode> {
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
fn get_forced_solve_reason_owned(params: &RunNodeParamsOwned) -> Option<String> {
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

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::unnecessary_literal_bound
)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_state_new() {
        let state = EngineState::new();
        assert_eq!(state.nodes_visited(), 0);
        assert_eq!(state.max_depth_seen(), 0);
    }

    #[test]
    fn test_engine_state_next_node_id() {
        let state = EngineState::new();
        let id1 = state.next_node_id();
        let id2 = state.next_node_id();
        assert_eq!(id1, "n1");
        assert_eq!(id2, "n2");
    }

    #[test]
    fn test_engine_state_record_visit() {
        let state = EngineState::new();
        state.record_visit(0);
        state.record_visit(2);
        state.record_visit(1);
        assert_eq!(state.nodes_visited(), 3);
        assert_eq!(state.max_depth_seen(), 2);
    }

    fn make_test_params(task: &str, depth: u32, max_depth: u32) -> RunNodeParamsOwned {
        let state = Arc::new(EngineState::new());
        let budget = Arc::new(RunBudget::new(1.0, 100_000, 5, 60_000));
        let token_counter = Arc::new(TokenCounter::new(100_000));
        let events = Arc::new(EventBus::new());
        let cache = Arc::new(ResultCache::default());
        let llm: Arc<dyn LlmBackend + Send + Sync> = Arc::new(MockLlm);
        let input = StartRunInput {
            max_depth,
            max_nodes: 50,
            ..Default::default()
        };
        RunNodeParamsOwned {
            task: task.to_string(),
            depth,
            lineage: Vec::new(),
            parent_id: None,
            input,
            state,
            budget,
            token_counter,
            events,
            cache,
            llm,
            router: Arc::new(Mutex::new(DepthRouter::new())),
            abort: Arc::new(AbortSignal::new()),
        }
    }

    #[test]
    fn test_get_forced_solve_reason_max_depth() {
        let params = make_test_params("task", 2, 2);
        let reason = get_forced_solve_reason_owned(&params);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("max depth"));
    }

    #[test]
    fn test_get_forced_solve_reason_cycle() {
        let state = Arc::new(EngineState::new());
        let budget = Arc::new(RunBudget::new(1.0, 100_000, 5, 60_000));
        let token_counter = Arc::new(TokenCounter::new(100_000));
        let events = Arc::new(EventBus::new());
        let cache = Arc::new(ResultCache::default());
        let llm: Arc<dyn LlmBackend + Send + Sync> = Arc::new(MockLlm);
        let input = StartRunInput::default();
        let params = RunNodeParamsOwned {
            task: "task A".to_string(),
            depth: 0,
            lineage: vec!["task a".to_string()],
            parent_id: None,
            input,
            state,
            budget,
            token_counter,
            events,
            cache,
            llm,
            router: Arc::new(Mutex::new(DepthRouter::new())),
            abort: Arc::new(AbortSignal::new()),
        };
        let reason = get_forced_solve_reason_owned(&params);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("cycle"));
    }

    #[test]
    fn test_get_forced_solve_reason_no_forcing() {
        let params = make_test_params("task", 0, 3);
        let reason = get_forced_solve_reason_owned(&params);
        assert!(reason.is_none());
    }

    struct MockLlm;

    #[async_trait::async_trait]
    impl arlm_llm::LlmBackend for MockLlm {
        async fn complete(
            &self,
            _req: arlm_llm::CompletionRequest,
        ) -> std::result::Result<arlm_llm::CompletionResponse, arlm_llm::LlmError> {
            Ok(arlm_llm::CompletionResponse {
                content: r#"{"action": "solve", "reason": "mock"}"#.to_string(),
                model: "mock".to_string(),
                usage: arlm_llm::UsageSummary::default(),
            })
        }
        fn name(&self) -> &str {
            "mock"
        }
        async fn health_check(&self) -> std::result::Result<(), arlm_llm::LlmError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_run_rlm_engine_mock() {
        let input = StartRunInput {
            run_id: Arc::from("test-run"),
            task: "test task".to_string(),
            max_depth: 1,
            max_nodes: 10,
            ..Default::default()
        };
        let llm: Arc<dyn LlmBackend + Send + Sync> = Arc::new(MockLlm);
        let result = run_rlm_engine(input, llm).await.expect("should succeed");
        assert_eq!(result.run_id, "test-run");
        assert!(!result.final_output.is_empty());
        assert!(result.stats.nodes_visited > 0);
    }
}
