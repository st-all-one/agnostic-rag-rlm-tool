//! Summarization RPCs: `TriggerSummarize`, `GetSummaryStatus`,
//! `StreamSummarizeProgress`.
//!
//! `TriggerSummarize` enqueues a job on the persistent summarization worker
//! and returns immediately; progress is streamed through the event hub.

use arlm_proto::proto::*;
use futures::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;
use tonic::{Response, Status};

use crate::grpc::EventStream;
use crate::grpc::error::{internal, not_found};
use crate::state::AppState;
use crate::store;
use crate::summarizer::{SummarizeJob, cost::estimate_cost};

/// Enqueue a summarization job for a project.
///
/// # Errors
///
/// Returns an error if the project is unknown or the job cannot be queued.
pub(crate) async fn handle_trigger_summarize(
    state: &AppState,
    req: SummarizeRequest,
) -> Result<Response<SummarizeResponse>, Status> {
    let run_id = uuid::Uuid::now_v7().to_string();

    let project_storage = state.storage.clone();
    let project = req.project.clone();
    let project_for_buffer = project.clone();
    let buffer_id = store::blocking(move || {
        store::buffer_id_for_project(&project_storage, &project_for_buffer)
    })
    .await
    .map_err(internal)?
    .ok_or_else(|| not_found("project not found"))?;

    let max_scope = i32::from(req.max_scope());
    let max_concurrent = if req.max_concurrent > 0 {
        req.max_concurrent
    } else {
        10
    };

    // Create the streaming channel for this run before enqueuing so
    // subscribers can attach immediately.
    let _rx = state.events.register_summarize(&run_id);

    let job = SummarizeJob {
        run_id: run_id.clone(),
        buffer_id,
        project: project.clone(),
        max_scope,
        max_concurrent: u32::try_from(max_concurrent).unwrap_or(10),
        force_refresh: req.force_refresh,
    };

    let _ = state.summarize_tx.send(job);

    let project_for_count = project.clone();
    let file_count = match store::summary_counts(&state.storage, &project_for_count) {
        Ok(c) => c.file,
        Err(e) => {
            tracing::warn!(project = %project, error = %e, "failed to estimate summarization cost");
            0
        }
    };
    let estimate = estimate_cost(u32::try_from(file_count).unwrap_or(0), 4, 0.01);

    tracing::info!(run_id, project = %project, buffer_id, "summarization enqueued");

    Ok(Response::new(SummarizeResponse {
        run_id,
        status: Some(SummarizeStatus {
            running: true,
            current_file: String::new(),
            files_remaining: i32::try_from(file_count).unwrap_or(i32::MAX),
            estimated_cost_usd: estimate.cost_usd,
        }),
    }))
}

/// Report the hierarchical summarization coverage for a project.
///
/// # Errors
///
/// Returns an error if storage access fails.
pub(crate) async fn handle_get_summary_status(
    state: &AppState,
    project: String,
) -> Result<Response<SummaryStatus>, Status> {
    let storage = state.storage.clone();
    let project_clone = project.clone();
    let result = store::blocking(move || {
        let counts = store::summary_counts(&storage, &project_clone)?;
        let buffer_id = store::buffer_id_for_project(&storage, &project_clone)?;
        let total_chunks = if let Some(bid) = buffer_id {
            storage.connection()?.execute(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM chunks WHERE buffer_id = ?1",
                    rusqlite::params![bid],
                    |row| row.get::<_, i64>(0),
                )?)
            })?
        } else {
            0
        };
        let _ = buffer_id;
        Ok((counts, total_chunks))
    })
    .await
    .map_err(internal)?;

    let (counts, total_chunks) = result;

    let coverage_ratio = if total_chunks > 0 {
        (counts.file as f32) / (total_chunks as f32)
    } else {
        0.0
    };

    tracing::info!(
        project = %project,
        total = counts.total,
        coverage = coverage_ratio,
        "summary status queried"
    );

    Ok(Response::new(SummaryStatus {
        project,
        total_chunks,
        summarized_chunks: counts.file,
        coverage_ratio,
        file_summaries: counts.file,
        module_summaries: counts.module,
        project_summaries: counts.project,
        last_updated: None,
        stale: Vec::new(),
    }))
}

/// Stream live progress for an active summarization run.
///
/// The client passes the `run_id` returned by `TriggerSummarize`.
///
/// # Errors
///
/// Returns an error if the channel cannot be registered.
pub(crate) fn handle_stream_summarize_progress(
    state: &AppState,
    run_id: String,
) -> Result<Response<EventStream<SummarizeProgress>>, Status> {
    let rx = state.events.register_summarize(&run_id);

    let stream = BroadcastStream::new(rx)
        .map(|item| item.map_err(|e| Status::internal(format!("summarize stream error: {e}"))));

    Ok(Response::new(Box::pin(stream)))
}
