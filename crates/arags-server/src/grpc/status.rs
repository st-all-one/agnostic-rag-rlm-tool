//! Server status RPC: `GetServerStatus`.

use tonic::{Response, Status};

use crate::state::AppState;
use crate::store;

use arags_proto::proto::ServerStatus;

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
        Ok((projects.len(), chunks))
    })
    .await
    .map_err(crate::grpc::error::internal)?;

    let (total_projects, total_chunks) = stats;

    tracing::info!(total_projects, total_chunks, "server status queried");

    Ok(Response::new(ServerStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: i32::try_from(state.uptime_seconds()).unwrap_or(i32::MAX),
        active_runs: 0,
        total_projects: i32::try_from(total_projects).unwrap_or(i32::MAX),
        total_chunks,
        write_queue: None,
    }))
}
