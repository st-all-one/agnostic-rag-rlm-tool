use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tracing::info;

use arlm_llm::LlmBackend;

use crate::budget::RunBudget;
use crate::cache::ResultCache;
use crate::events::{EventBus, EventSink, RlmEvent};
use crate::logging::ScopedTimer;
use crate::memory::MemoryProvider;
use crate::router::DepthRouter;
use crate::token_counter::TokenCounter;
use crate::types::{RlmRunResult, RunStats, StartRunInput, now_ms};

pub mod compactor;
pub mod node;
pub mod state;

pub use crate::engine::compactor::RootCompactor;
pub use crate::engine::node::get_forced_solve_reason_owned;
pub use crate::engine::node::RunNodeParamsOwned;
pub use crate::engine::state::EngineState;
use crate::engine::node::run_node_owned;

/// Run the RLM engine on a task with an internal event bus.
///
/// # Errors
/// Returns an error if the engine encounters a fatal failure.
#[allow(clippy::cast_possible_truncation)]
pub async fn run_rlm_engine(
    input: StartRunInput,
    llm: Arc<dyn LlmBackend + Send + Sync>,
) -> Result<RlmRunResult> {
    run_rlm_engine_with_events(input, llm, EventBus::new(), None).await
}

/// Run the RLM engine on a task, broadcasting `RlmEvent`s on the given bus.
///
/// When `memory` is `Some`, the run's trajectory is persisted via
/// [`MemoryProvider::save_trajectory`] once the run completes.
///
/// # Errors
/// Returns an error if the engine encounters a fatal failure.
#[allow(clippy::cast_possible_truncation)]
pub async fn run_rlm_engine_with_events(
    input: StartRunInput,
    llm: Arc<dyn LlmBackend + Send + Sync>,
    event_bus: EventBus,
    memory: Option<Arc<dyn MemoryProvider>>,
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
    let sink = EventSink::new(events.clone());
    let cache = Arc::new(ResultCache::default_config());
    let router = Arc::new(parking_lot::Mutex::new(DepthRouter::new()));

    sink.emit(RlmEvent::RunStart {
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
        memory: memory.clone(),
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

    sink.emit(RlmEvent::RunEnd {
        run_id: input.run_id.clone(),
        duration_ms,
        nodes_visited: run_stats.nodes_visited,
    });

    let result = RlmRunResult {
        run_id: input.run_id.to_string(),
        backend: input.backend.to_string(),
        final_output,
        root,
        stats: run_stats,
    };

    // Persist trajectory if a memory provider is configured (#3).
    if let Some(provider) = &memory {
        if let Err(e) = provider.save_trajectory(&input, &result) {
            tracing::warn!(error = %e, "failed to persist trajectory to memory");
        }
    }

    info!(
        run_id = input.run_id.as_ref(),
        nodes = state.nodes_visited(),
        max_depth = state.max_depth_seen(),
        duration_ms,
        "RLM run completed"
    );

    Ok(result)
}
