//! Server status RPCs: `GetServerStatus`, `StreamEvents`.

use arlm_proto::proto::*;
use tokio_stream::wrappers::BroadcastStream;
use futures::StreamExt as _;
use tonic::{Response, Status};

use crate::grpc::error::stream_error;
use crate::grpc::EventStream;
use crate::state::AppState;
use crate::store;

/// Report server and storage statistics.
///
/// # Errors
///
/// Returns an error if storage access fails.
pub(crate) async fn handle_get_server_status(
    state: &AppState,
) -> Result<Response<ServerStatus>, Status> {
    let storage = state.storage.clone();
    let stats = store::blocking(move || {
        let projects = store::list_projects(&storage)?;
        let chunks: i64 = storage
            .connection()?
            .execute(|conn| {
                conn.query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
                    .map_err(anyhow::Error::from)
            })?;
        let summaries = store::count_all_summaries(&storage)?;
        let active_runs = store::count_active_runs(&storage)?;
        Ok((projects.len(), chunks, summaries, active_runs))
    })
    .await
    .map_err(crate::grpc::error::internal)?;

    let (total_projects, total_chunks, total_summaries, active_runs) = stats;

    let write_queue_stats = state.write_queue.stats();
    let write_queue = Some(WriteQueueStats {
        pending_writes: i32::try_from(write_queue_stats.pending).unwrap_or(i32::MAX),
        batched_last_flush: i32::try_from(write_queue_stats.flushed).unwrap_or(i32::MAX),
        avg_latency_ms: 0.0,
    });

    tracing::info!(
        total_projects,
        total_chunks,
        total_summaries,
        active_runs,
        "server status queried"
    );

    Ok(Response::new(ServerStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: i32::try_from(state.uptime_seconds()).unwrap_or(i32::MAX),
        active_runs: i32::try_from(active_runs).unwrap_or(i32::MAX),
        total_projects: i32::try_from(total_projects).unwrap_or(i32::MAX),
        total_chunks,
        total_summaries,
        write_queue,
        summarize: None,
    }))
}

/// Stream every event the server emits (runs and summarization).
///
/// # Errors
///
/// Returns an error if the event channel cannot be subscribed.
pub(crate) fn handle_stream_events(
    state: &AppState,
) -> Result<Response<EventStream<RunEvent>>, Status> {
    let rx = state.events.subscribe_all();

    let stream = BroadcastStream::new(rx).map(|item| match item {
        Ok(event) => match event {
            crate::events::ServerEvent::Run(ev) => Ok(ev),
            crate::events::ServerEvent::Summarize(ev) => {
                let data = serde_json::json!({
                    "run_id": ev.run_id,
                    "current_file": ev.current_file,
                    "completed": ev.completed,
                    "total": ev.total,
                })
                .to_string();
                Ok(RunEvent {
                    run_id: ev.run_id,
                    event_type: "summarize_progress".to_string(),
                    data,
                    timestamp: None,
                })
            }
        },
        Err(e) => Err(stream_error(e)),
    });

    Ok(Response::new(Box::pin(stream)))
}