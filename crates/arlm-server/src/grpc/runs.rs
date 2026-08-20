//! RLM run RPCs: `StartRun`, `GetRun`, `CancelRun`, `StreamRun`.
//!
//! `StartRun` persists the run in the `running` state, spawns the RLM engine
//! on the background runtime, and bridges engine events onto the event hub so
//! clients can stream live progress. `CancelRun` signals the engine's abort
//! flag so the run can stop cooperatively.

use std::sync::Arc;

use arlm_core::{RlmBackend, RlmEvent, RlmMode, StartRunInput, run_rlm_engine_with_events};
use arlm_proto::proto::*;
use tokio_stream::wrappers::BroadcastStream;
use futures::StreamExt as _;
use tonic::{Response, Status};

use crate::grpc::error::{internal, not_found};
use crate::grpc::EventStream;
use crate::state::AppState;
use crate::store;
use crate::timing::Timer;

/// Start a new RLM run in the background.
///
/// # Errors
///
/// Returns an error if storage access fails or the request is invalid.
pub(crate) async fn handle_start_run(
    state: &AppState,
    req: RunRequest,
) -> Result<Response<RunResponse>, Status> {
    if req.project.trim().is_empty() || req.task.trim().is_empty() {
        return Err(Status::invalid_argument("project and task are required"));
    }

    let run_id = uuid::Uuid::now_v7().to_string();
    let backend = req.backend.to_lowercase();
    let model = if req.model.trim().is_empty() {
        None
    } else {
        Some(req.model.clone())
    };

    let input = build_run_input(&run_id, &req.project, &req.task, &backend, model.clone(), req.options)
        .map_err(|e| Status::invalid_argument(format!("invalid run options: {e}")))?;

    let run = store::RunRow {
        id: run_id.clone(),
        project: Some(req.project.clone()),
        task: req.task.clone(),
        backend: Some(backend.clone()),
        model,
        status: "running".to_string(),
        answer: None,
        started_at: Some(chrono::Utc::now().timestamp()),
        finished_at: None,
        duration_ms: None,
        total_tokens: 0,
        total_cost: 0.0,
        nodes_visited: 0,
        max_depth: 0,
    };

    let storage = state.storage.clone();
    store::blocking(move || store::insert_run(&storage, &run))
        .await
        .map_err(internal)?;

    spawn_engine(state.clone(), run_id.clone(), input);

    tracing::info!(run_id, project = %req.project, backend = %backend, "run started");

    Ok(Response::new(RunResponse {
        run_id,
        status: RunStatus::StatusRunning.into(),
    }))
}

/// Spawn the RLM engine for a registered run, bridging events onto the hub.
fn spawn_engine(state: AppState, run_id: String, input: StartRunInput) {
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

        let result = run_rlm_engine_with_events(input, llm, bus).await;

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
fn build_run_input(
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

/// Fetch the current state of a run.
///
/// # Errors
///
/// Returns an error if storage access fails or the run is unknown.
pub(crate) async fn handle_get_run(
    state: &AppState,
    run_id: String,
) -> Result<Response<RunResult>, Status> {
    let storage = state.storage.clone();
    let run_id_clone = run_id.clone();
    let row = store::blocking(move || store::get_run(&storage, &run_id_clone))
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("run not found"))?;

    let stats = RunStats {
        nodes_visited: i32::try_from(row.nodes_visited).unwrap_or(i32::MAX),
        max_depth_reached: i32::try_from(row.max_depth).unwrap_or(i32::MAX),
        total_tokens: 0,
        total_cost_usd: 0.0,
        duration_ms: row.duration_ms.unwrap_or(0) as f64,
    };

    Ok(Response::new(RunResult {
        run_id,
        status: (store::proto_run_status(&row.status) as i32),
        answer: row.answer.unwrap_or_default(),
        stats: Some(stats),
    }))
}

/// Cancel an active run.
///
/// # Errors
///
/// Returns an error if storage access fails.
pub(crate) async fn handle_cancel_run(
    state: &AppState,
    run_id: String,
) -> Result<Response<()>, Status> {
    if !state.abort_run(&run_id) {
        // Not tracked as active; still update the persisted record.
    }

    let storage = state.storage.clone();
    let run_id_clone = run_id.clone();
    store::blocking(move || store::cancel_run(&storage, &run_id_clone))
        .await
        .map_err(internal)?;

    tracing::info!(run_id, "run cancellation requested");
    Ok(Response::new(()))
}

/// Stream live events for a single run.
///
/// # Errors
///
/// Returns an error if the run channel cannot be registered.
pub(crate) fn handle_stream_run(
    state: &AppState,
    run_id: String,
) -> Result<Response<EventStream<RunEvent>>, Status> {
    let rx = state.events.register_run(&run_id);

    let stream = BroadcastStream::new(rx)
        .map(|item| item.map_err(|e| Status::internal(format!("run stream error: {e}"))));

    Ok(Response::new(Box::pin(stream)))
}