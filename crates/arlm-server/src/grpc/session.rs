//! Session RPCs: `CreateSession`, `ListSessions`, `GetSession`, `AddSessionTurn`.
//!
//! Sessions map to the `sessions` table; turns are stored in `session_history`
//! as query/result pairs.

use arlm_proto::proto::*;
use tonic::{Response, Status};

use crate::grpc::error::{internal, not_found};
use crate::state::AppState;
use crate::store;

fn session_to_info(state: &AppState, row: &store::SessionRow) -> SessionInfo {
    let turn_count = match store::count_session_turns(&state.storage, &row.id) {
        Ok(n) => i32::try_from(n).unwrap_or(i32::MAX),
        Err(e) => {
            tracing::warn!(session_id = %row.id, error = %e, "failed to count session turns");
            0
        }
    };

    SessionInfo {
        session_id: row.id.clone(),
        project: row.project.clone(),
        title: row.title.clone(),
        created_at: row.created_at.map(ts),
        turn_count,
    }
}

/// Create a session for a project.
///
/// # Errors
///
/// Returns an error if storage access fails.
pub(crate) async fn handle_create_session(
    state: &AppState,
    req: CreateSessionRequest,
) -> Result<Response<SessionInfo>, Status> {
    let session_id = uuid::Uuid::now_v7().to_string();
    let project = req.project.clone();
    let title = req.title.clone();

    let storage = state.storage.clone();
    let session_id_clone = session_id.clone();
    let project_clone = project.clone();
    let title_clone = title.clone();
    store::blocking(move || {
        store::insert_session(&storage, &session_id_clone, &project_clone, &title_clone)
    })
    .await
    .map_err(internal)?;

    let row = store::SessionRow {
        id: session_id.clone(),
        project,
        title,
        created_at: Some(chrono::Utc::now().timestamp()),
        updated_at: None,
    };

    tracing::info!(session_id = %session_id, "session created");

    Ok(Response::new(session_to_info(state, &row)))
}

/// List sessions for a project.
///
/// # Errors
///
/// Returns an error if storage access fails.
pub(crate) async fn handle_list_sessions(
    state: &AppState,
    project: String,
) -> Result<Response<ListSessionsResponse>, Status> {
    let storage = state.storage.clone();
    let project_clone = project.clone();
    let rows = store::blocking(move || store::list_sessions(&storage, &project_clone))
        .await
        .map_err(internal)?;

    let sessions = rows.iter().map(|r| session_to_info(state, r)).collect();

    Ok(Response::new(ListSessionsResponse { sessions }))
}

/// Fetch a single session.
///
/// # Errors
///
/// Returns an error if storage access fails or the session is unknown.
pub(crate) async fn handle_get_session(
    state: &AppState,
    session_id: String,
) -> Result<Response<SessionInfo>, Status> {
    let storage = state.storage.clone();
    let session_id_clone = session_id.clone();
    let row = store::blocking(move || store::get_session(&storage, &session_id_clone))
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("session not found"))?;

    Ok(Response::new(session_to_info(state, &row)))
}

/// Add a turn to a session.
///
/// # Errors
///
/// Returns an error if storage access fails.
pub(crate) async fn handle_add_session_turn(
    state: &AppState,
    req: AddSessionTurnRequest,
) -> Result<Response<SessionTurn>, Status> {
    let storage = state.storage.clone();
    let session_id = req.session_id.clone();
    let query = req.query.clone();
    let response = req.response.clone();

    store::blocking(move || {
        store::insert_session_turn(&storage, &session_id, &query, &response)
    })
    .await
    .map_err(internal)?;

    Ok(Response::new(SessionTurn {
        query: req.query,
        response: req.response,
        timestamp: Some(ts(chrono::Utc::now().timestamp())),
    }))
}

fn ts(seconds: i64) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds,
        nanos: 0,
    }
}