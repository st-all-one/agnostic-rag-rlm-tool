use std::time::Instant;

use anyhow::{Context, Result};
use arlm_core::events::RlmEvent;
use serde_json::Value;
use tracing::{debug, instrument, warn};

use crate::commands::serve::state::AppState;
use crate::util::data_dir;

/// Extract the run id from any RLM event.
#[must_use]
pub fn extract_run_id(event: &RlmEvent) -> &str {
    match event {
        RlmEvent::RunStart { run_id, .. }
        | RlmEvent::NodeStart { run_id, .. }
        | RlmEvent::NodePlan { run_id, .. }
        | RlmEvent::NodeSolve { run_id, .. }
        | RlmEvent::NodeSynthesize { run_id, .. }
        | RlmEvent::CostUpdate { run_id, .. }
        | RlmEvent::CacheHit { run_id, .. }
        | RlmEvent::NodeEnd { run_id, .. }
        | RlmEvent::RunEnd { run_id, .. } => run_id,
    }
}

/// List all indexed projects.
///
/// # Errors
/// Returns an error if the storage backend cannot be opened or listing buffers
/// fails.
#[instrument(skip_all)]
pub fn handle_status_all(_state: &AppState) -> Result<Value> {
    let start = Instant::now();

    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;

    let buffers = storage.list_buffers().context("failed to list buffers")?;

    let mut items: Vec<Value> = Vec::with_capacity(buffers.len());
    for b in &buffers {
        items.push(serde_json::json!({
            "id": b.id,
            "name": b.name,
            "path": b.path,
            "total_chunks": b.total_chunks,
            "total_files": b.total_files,
            "last_indexed_at": b.last_indexed_at,
        }));
    }

    debug!(elapsed_ms = %start.elapsed().as_millis(), "status all");
    Ok(serde_json::json!({
        "projects": items,
        "count": buffers.len(),
    }))
}

/// Return the status of a specific run by id.
#[instrument(skip_all)]
pub fn handle_status_by_id(_state: &AppState, run_id: &str) -> Value {
    let start = Instant::now();

    let storage = match arlm_storage::Storage::open(&data_dir()) {
        Ok(s) => s,
        Err(e) => {
            warn!(run_id = %run_id, error = %e, "failed to open storage");
            return serde_json::json!({
                "run_id": run_id,
                "status": "error",
                "message": format!("Failed to open storage: {e}"),
            });
        }
    };

    let result = match storage.get_run(run_id) {
        Ok(Some(run)) => {
            let usage = storage.get_run_model_usage(run_id).unwrap_or_default();
            let models: Vec<Value> = usage
                .iter()
                .map(|u| {
                    serde_json::json!({
                        "model": u.model,
                        "calls": u.calls,
                        "input_tokens": u.input_tokens,
                        "output_tokens": u.output_tokens,
                        "cost": u.cost,
                    })
                })
                .collect();

            serde_json::json!({
                "run_id": run.id,
                "task": run.task,
                "status": run.status,
                "agent": run.agent,
                "started_at": run.started_at,
                "finished_at": run.finished_at,
                "duration_ms": run.duration_ms,
                "total_cost": run.total_cost,
                "total_tokens": run.total_tokens,
                "models": models,
            })
        }
        Ok(None) => {
            serde_json::json!({
                "run_id": run_id,
                "status": "not_found",
                "message": "Run not found",
            })
        }
        Err(e) => {
            serde_json::json!({
                "run_id": run_id,
                "status": "error",
                "message": format!("Failed to query run: {e}"),
            })
        }
    };

    debug!(elapsed_ms = %start.elapsed().as_millis(), run_id = %run_id, "status by id");
    result
}
