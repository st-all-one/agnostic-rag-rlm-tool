//! Per-user query history RPC (plan 019, E).
//!
//! `GetHistory` is scoped by the authenticated caller: admins may query any
//! `user` (or all users when `user` is empty); non-admins are forced to their
//! own `username`. Without a session the call returns `UNAUTHENTICATED`.

use tonic::{Request, Response, Status};

use crate::auth;
use crate::grpc::error::internal;
use crate::state::AppState;
use crate::store;

use arags_proto::proto::{GetHistoryRequest, GetHistoryResponse, HistoryEntry};

/// Fetch per-user query history (auth-scoped).
///
/// # Errors
///
/// Returns `UNAUTHENTICATED` without a session, or `internal` on storage failure.
pub async fn handle_get_history(
    state: &AppState,
    request: Request<GetHistoryRequest>,
) -> Result<Response<GetHistoryResponse>, Status> {
    // No session → unauthenticated (plan 019, E).
    let ctx = auth::authenticate(request.metadata(), &state.storage)?;

    let req = request.into_inner();

    // Scope: admin may read any user (or all); others are pinned to themselves.
    let user_filter: Option<String> = if ctx.is_admin() {
        if req.user.trim().is_empty() {
            None
        } else {
            Some(req.user.trim().to_string())
        }
    } else {
        Some(ctx.username.clone())
    };

    let limit = if req.limit > 0 { req.limit } else { 50 };

    let storage = state.storage.clone();
    let records = store::blocking(move || {
        let mgr = arags_memory::HistoryManager::new(storage);
        mgr.recent_opt_user(user_filter.as_deref(), limit)
    })
    .await
    .map_err(internal)?;

    let entries = records
        .into_iter()
        .map(|r| HistoryEntry {
            id: r.id,
            user: r.user.unwrap_or_default(),
            question: r.query,
            created_at: r.created_at.to_string(),
            cache_id: String::new(),
        })
        .collect();

    Ok(Response::new(GetHistoryResponse { entries }))
}
