//! Background RLM engine bridge for `StartRun`.
//!
//! Owns the tokio task that drives [`arlm_core::run_rlm_engine_with_events`],
//! translates engine events into proto events for the event hub, and persists
//! the final result (or failure) back to storage.

use std::sync::Arc;

use arlm_core::{RlmBackend, RlmEvent, RlmMode, StartRunInput, run_rlm_engine_with_events};
use arlm_proto::proto::*;

use crate::state::AppState;
use crate::store;
use crate::timing::Timer;

/// Spawn the RLM engine for a registered run, bridging events onto the hub.
pub(crate) fn spawn_engine(state: AppState, run_id: String, input: StartRunInput) {
    // Keep the abort signal in sync in case the run is killed immediately.
    state.register_abort(&run_id);

    let storage = state.storage.clone();
    let hub = state.events.clone();
    let llm = state.llm.clone();

    tokio::spawn(async move {
        let _timer = Timer::new("rlm_engine_run");

        let bus = arlm_core::EventBus::new();
        let mut bus_rx = bus.subscribe();

        // Bridge engine events on to the shared hub (own clone for the task).
        let hub_bridge = hub.clone();
        let bridge = tokio::spawn(async move {
            while let Ok(event) = bus_rx.recv().await {
                let proto_event = proto_event(&event);
                hub_bridge.publish_run(proto_event);
            }
        });

        let result = run_rlm_engine_with_events(input, llm, bus, None).await;

        bridge.abort();
        state.release_run(&run_id);
        hub.unregister_run(&run_id);

        match result {
            Ok(res) => {
                let storage_clone = storage.clone();
                let run_id_clone = run_id.clone();
                if let Err(e) = store::blocking(move || {
                    store::complete_run(
                        &storage_clone,
                        &run_id_clone,
                        &res.final_output,
                        res.stats.duration_ms,
                        res.stats.nodes_visited,
                        res.stats.max_depth_seen,
                        0,
                        0.0,
                    )
                })
                .await
                {
                    tracing::error!(run_id = %run_id, error = %e, "failed to persist completed run");
                }
                tracing::info!(
                    run_id = %run_id,
                    duration_ms = res.stats.duration_ms,
                    nodes = res.stats.nodes_visited,
                    "run completed"
                );
            }
            Err(e) => {
                let err = e.to_string();
                let storage_clone = storage.clone();
                let run_id_clone = run_id.clone();
                let err_clone = err.clone();
                if let Err(persist_err) = store::blocking(move || {
                    store::fail_run(&storage_clone, &run_id_clone, &err_clone)
                })
                .await
                {
                    tracing::error!(run_id = %run_id, error = %persist_err, "failed to persist failed run");
                }
                tracing::error!(run_id = %run_id, error = %err, "run failed");
            }
        }
    });
}

/// Translate an engine `RlmEvent` into a proto `RunEvent`.
fn proto_event(ev: &RlmEvent) -> RunEvent {
    let (run_id, event_type) = match ev {
        RlmEvent::RunStart { run_id, .. } => (run_id.as_ref(), "run_start"),
        RlmEvent::NodeStart { run_id, .. } => (run_id.as_ref(), "node_start"),
        RlmEvent::NodePlan { run_id, .. } => (run_id.as_ref(), "node_plan"),
        RlmEvent::NodeSolve { run_id, .. } => (run_id.as_ref(), "node_solve"),
        RlmEvent::NodeSynthesize { run_id, .. } => (run_id.as_ref(), "node_synthesize"),
        RlmEvent::CostUpdate { run_id, .. } => (run_id.as_ref(), "cost_update"),
        RlmEvent::CacheHit { run_id, .. } => (run_id.as_ref(), "cache_hit"),
        RlmEvent::NodeEnd { run_id, .. } => (run_id.as_ref(), "node_end"),
        RlmEvent::RunEnd { run_id, .. } => (run_id.as_ref(), "run_end"),
    };

    RunEvent {
        run_id: run_id.to_string(),
        event_type: event_type.to_string(),
        data: serde_json::to_string(ev).unwrap_or_default(),
        timestamp: Some(prost_types::Timestamp {
            seconds: chrono::Utc::now().timestamp(),
            nanos: 0,
        }),
    }
}

/// Build the engine input from a run request.
pub(crate) fn build_run_input(
    run_id: &str,
    project: &str,
    task: &str,
    backend: &str,
    model: Option<String>,
    opts: Option<RunOptions>,
) -> anyhow::Result<StartRunInput> {
    let mut input = StartRunInput::default();
    input.run_id = Arc::from(run_id);
    input.task = task.to_string();
    input.project = project.to_string();
    input.backend = parse_backend(backend);
    input.mode = RlmMode::Auto;
    input.model = model;
    input.agent = "arlm-server".to_string();

    if let Some(o) = opts {
        if o.max_depth > 0 {
            input.max_depth = u32::try_from(o.max_depth).unwrap_or(u32::MAX);
        }
        if o.max_iterations > 0 {
            input.max_nodes = u32::try_from(o.max_iterations).unwrap_or(u32::MAX);
        }
        if o.max_budget_usd > 0.0 {
            input.max_budget = f64::from(o.max_budget_usd);
        }
        if o.max_timeout_seconds > 0.0 {
            input.timeout_ms = u64::try_from((f64::from(o.max_timeout_seconds) * 1000.0) as u128)
                .unwrap_or(u64::MAX);
        }
        if o.max_tokens > 0 {
            input.max_tokens = u64::try_from(o.max_tokens).unwrap_or(u64::MAX);
        }
    }

    Ok(input)
}

fn parse_backend(backend: &str) -> RlmBackend {
    match backend {
        "anthropic" | "claude" => RlmBackend::Anthropic,
        "gemini" | "google" => RlmBackend::Gemini,
        "ollama" | "local" => RlmBackend::Ollama,
        "deepseek" => RlmBackend::DeepSeek,
        "mimo" => RlmBackend::MiMo,
        _ => RlmBackend::OpenAi,
    }
}
