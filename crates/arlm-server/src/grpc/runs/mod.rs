//! RLM run RPCs: `StartRun`, `GetRun`, `CancelRun`, `StreamRun`.
//!
//! `StartRun` persists the run in the `running` state, spawns the RLM engine on
//! the background runtime (see [`engine`]), and bridges engine events onto the
//! event hub so clients can stream live progress. `CancelRun` signals the
//! engine's abort flag so the run can stop cooperatively.

pub mod engine;

use arlm_proto::proto::*;
use futures::StreamExt as _;
use tonic::{Response, Status};

use crate::grpc::error::{internal, not_found};
use crate::state::AppState;
use crate::store;

use engine::spawn_engine;

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

    let input = engine::build_run_input(
        &run_id,
        &req.project,
        &req.task,
        &backend,
        model.clone(),
        req.options,
    )
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
) -> Result<Response<crate::grpc::EventStream<RunEvent>>, Status> {
    let rx = state.events.register_run(&run_id);

    let stream = tokio_stream::wrappers::BroadcastStream::new(rx)
        .map(|item| item.map_err(|e| Status::internal(format!("run stream error: {e}"))));

    Ok(Response::new(Box::pin(stream)))
}
