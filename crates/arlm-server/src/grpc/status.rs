//! Server status RPC: `GetServerStatus`.

use arlm_proto::proto::*;
use tonic::{Response, Status};

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
        let chunks: i64 = storage.connection()?.execute(|conn| {
            conn.query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
                .map_err(anyhow::Error::from)
        })?;
        let summaries = store::count_all_summaries(&storage)?;
        Ok((projects.len(), chunks, summaries))
    })
    .await
    .map_err(crate::grpc::error::internal)?;

    let (total_projects, total_chunks, total_summaries) = stats;

    tracing::info!(
        total_projects,
        total_chunks,
        total_summaries,
        "server status queried"
    );

    Ok(Response::new(ServerStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: i32::try_from(state.uptime_seconds()).unwrap_or(i32::MAX),
        active_runs: 0,
        total_projects: i32::try_from(total_projects).unwrap_or(i32::MAX),
        total_chunks,
        total_summaries,
        write_queue: None,
        summarize: None,
    }))
}
